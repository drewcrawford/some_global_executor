/*!
A wasm-friendly static executor.
*/
use js_sys::{Function, Reflect};
use some_executor::observer::{Observer, ObserverNotified};
use some_executor::static_support::OwnedSomeStaticExecutorErasingNotifier;
use some_executor::task::Task;
use some_executor::{
    BoxedStaticObserver, BoxedStaticObserverFuture, DynStaticExecutor, ObjSafeStaticTask,
    SomeStaticExecutor,
};
use std::cell::RefCell;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

#[derive(Clone, Debug)]
pub struct StaticExecutor {
    close_info: Rc<RefCell<CloseInfo>>,
}

impl StaticExecutor {
    pub fn new() -> Self {
        StaticExecutor {
            close_info: Rc::new(RefCell::new(CloseInfo::new())),
        }
    }
}

#[derive(Debug)]
pub struct CloseInfo {
    running_tasks: usize,
    installed_close_handler: Option<js_sys::Function>,
    close_is_called: bool,
}

impl CloseInfo {
    fn new() -> Self {
        CloseInfo {
            running_tasks: 0,
            installed_close_handler: None,
            close_is_called: false,
        }
    }
}

/**
Patches the worker close function.

# Here be dragons!

Let me explain WTF is going on here.  When we execute futures with [wasm_bindgen_futures::spawn_local],
they need to be run by the event loop. Which means the event loop needs to um, actually run.

You may have some ideas about that like setTimeout, or requestAnimationFrame, but actually what happens
is when your thread is done, wasm_thread will call `close` on the worker, which will
cause the worker to exit immediately, without running any pending tasks and with no log or indication
of what happened.  Then you sit around wondering why your tasks are not running and trace them
through 20 layers of dependencies.

How do I know?  This is the kind of bug that I spent 3 days trying to figure out two years ago,
promptly forgot about, and then spent another 3 days trying to figure out again.

## The solution

We patch the `close` function on the global object to do two things:
1. Set a flag that close was called, so we can check it later.
2. Check if there are any running tasks. If there are, we do nothing and let the event loop run.
   If there are no running tasks, we call the original `close` function to actually close the worker.

This way, we ensure that the worker does not exit prematurely and all tasks are executed before the worker is closed.

Later when the last task is done, we call the original `close` function to actually close the worker.

## Alternatives considered

The usual hack is to use [wasm_bindgen::throw_str] to reject the close call, but that
is not a great idea as it shows up in console and prevents worker cleanup.
See https://github.com/rustwasm/wasm-bindgen/issues/2945 and
https://github.com/chemicstry/wasm_thread/issues/6.

I sent a PR to wasm_bindgen to document spawn_local's behavior to at least warn people,
but it was not merged.  See https://github.com/rustwasm/wasm-bindgen/pull/4391.

wasm_thread has some intention to "support async thread entrypoints"
which "would probably need a major rewrite", see https://github.com/chemicstry/wasm_thread/issues/10.

I am not optimistic about that because it's been several years with no real progress on that front.

There are two stale PRs in a related area that seem to mostly be stuck because any year now
we'll rewrite wasm_thread to support async thread entrypoints:

* https://github.com/chemicstry/wasm_thread/issues/10
* https://github.com/chemicstry/wasm_thread/pull/18

At this point I've lost enough time on this bug to actually consider implementing it myself,
but I'm not sure they have the review bandwidth to look at it, and this is simpler.
*/

fn patch_if_needed(close_info: &Rc<RefCell<CloseInfo>>) {
    if close_info.borrow().installed_close_handler.is_some() {
        // Already patched, do nothing
        return;
    } else {
        let global = js_sys::global();

        // 2. Grab the original `close` function (`fn () -> !`)
        let orig_close: Function = Reflect::get(&global, &"close".into())
            .expect("global.close should exist")
            .dyn_into()
            .expect("global.close is not a function");

        // 3. Make a Rust closure that logs *then* calls the real close
        let move_close_info = close_info.clone();
        let wrapper = Closure::wrap(Box::new(move || {
            //move our close_info into the closure itself.
            let mut borrow_mut = move_close_info.borrow_mut();
            borrow_mut.close_is_called = true;
            consider_closing(&mut *borrow_mut)
        }) as Box<dyn Fn()>);

        // 4. Replace `self.close` with our wrapper
        Reflect::set(&global, &"close".into(), wrapper.as_ref().unchecked_ref())
            .expect("failed to patch close");

        // 5. Prevent the wrapper from being dropped
        wrapper.forget();
        close_info.borrow_mut().installed_close_handler = Some(orig_close);
    }
}

fn consider_closing(info: &mut CloseInfo) {
    if !info.close_is_called {
        // If close was not called, we do nothing
        return;
    }
    let count = info.running_tasks;
    if count > 0 {
        // If there are still running tasks, we do nothing
        return;
    }
    // If no tasks are running, we can safely call the original close
    if let Some(orig_close) = info.installed_close_handler.as_mut().take() {
        orig_close
            .call0(&js_sys::global())
            .expect("Failed to call original close");
    }
}

struct WasmFuture<F> {
    future: F,
    close_info: Rc<RefCell<CloseInfo>>,
}

impl<F: Future> Future for WasmFuture<F> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let (future, close_info) = unsafe {
            let s = self.get_unchecked_mut();
            (
                Pin::new_unchecked(&mut s.future),
                Pin::new(&mut s.close_info),
            )
        };
        let r = future.poll(cx);
        match r {
            Poll::Pending => Poll::Pending,
            Poll::Ready(output) => {
                //decrement the running tasks
                close_info.borrow_mut().running_tasks -= 1;
                consider_closing(&mut close_info.borrow_mut());
                Poll::Ready(output)
            }
        }
    }
}

impl StaticExecutor {
    pub fn spawn<F>(&self, fut: F)
    where
        F: Future<Output = ()> + 'static,
    {
        patch_if_needed(&self.close_info);
        //increment the running tasks
        self.close_info.borrow_mut().running_tasks += 1;
        let t = WasmFuture {
            future: fut,
            close_info: self.close_info.clone(),
        };
        wasm_bindgen_futures::spawn_local(t);
    }
}

impl SomeStaticExecutor for StaticExecutor {
    type ExecutorNotifier = Infallible;

    fn spawn_static<F, Notifier: ObserverNotified<F::Output>>(
        &mut self,
        task: Task<F, Notifier>,
    ) -> impl Observer<Value = F::Output>
    where
        Self: Sized,
        F: Future + 'static,
        F::Output: 'static + Unpin,
    {
        let (spawned, observer) = task.spawn_static(self);
        self.spawn(async {
            spawned.into_future().await; //swallow return value
        });
        observer
    }

    fn spawn_static_async<F, Notifier: ObserverNotified<F::Output>>(
        &mut self,
        task: Task<F, Notifier>,
    ) -> impl Future<Output = impl Observer<Value = F::Output>>
    where
        Self: Sized,
        F: Future + 'static,
        F::Output: 'static + Unpin,
    {
        async move { self.spawn_static(task) }
    }

    fn spawn_static_objsafe(&mut self, task: ObjSafeStaticTask) -> BoxedStaticObserver {
        Box::new(self.spawn_static(task))
    }

    fn spawn_static_objsafe_async<'s>(
        &'s mut self,
        task: ObjSafeStaticTask,
    ) -> BoxedStaticObserverFuture<'s> {
        Box::new(async {
            let o = self.spawn_static_objsafe(task);
            o
        })
    }

    fn clone_box(&self) -> Box<DynStaticExecutor> {
        let adapt = OwnedSomeStaticExecutorErasingNotifier::new(self.clone());
        Box::new(adapt)
    }

    fn executor_notifier(&mut self) -> Option<Self::ExecutorNotifier> {
        None
    }
}

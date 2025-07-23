use std::any::Any;
use std::convert::Infallible;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use atomic_waker::AtomicWaker;
use logwise::{debuginternal_sync};
use some_executor::DynExecutor;
use some_executor::observer::{Observer, ObserverNotified};
use some_executor::task::{DynSpawnedTask, Task};
use some_executor::SomeExecutor;

#[derive(Debug,PartialEq, Eq, Hash,Clone)]
pub struct Executor {
    imp: sys::Executor,
    name: String,
}

impl Executor {
    pub fn new(name: String, threads: usize) -> Self {
        let name_clone = name.clone();
        let inner = sys::Executor::new(name, threads);
        Executor {
            imp: inner,
            name: name_clone,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/**
A drain operation that waits for all tasks to finish.
*/
pub struct ExecutorDrain {
    executor: Executor,
}

impl Future for ExecutorDrain {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        //need to register prior to check
        let drain_notify = self.executor.imp.drain_notify();
        drain_notify.waker.register(cx.waker());
        let running_tasks = drain_notify.running_tasks.load(std::sync::atomic::Ordering::Relaxed);
        debuginternal_sync!("ExecutorDrain::poll running_tasks={running_tasks}",running_tasks=(running_tasks as u64));
        if running_tasks == 0 {
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    }
}

#[derive(Debug)]
struct DrainNotify {
    running_tasks: AtomicUsize,
    waker: AtomicWaker
}

impl DrainNotify {
    fn new() -> Self {
        Self {
            running_tasks: AtomicUsize::new(0),
            waker: AtomicWaker::new(),
        }
    }
}

mod waker;
mod sys;







#[derive(Debug)]
struct SpawnedTask {
    imp: sys::SpawnedTask,
}


impl SpawnedTask {
    fn new(task: Box<dyn DynSpawnedTask<Infallible>>) -> Self {
        Self {
            imp: sys::SpawnedTask::new(task),
        }
    }
    
}


impl Executor {


    pub fn new_default() -> Self {
        Self::new("default".to_string(), num_cpus::get())
    }




    pub fn drain(self) {
        let _interval = logwise::perfwarn_begin!("Executor::drain busyloop");
        loop {
            let running_tasks = self.imp.drain_notify().running_tasks.load(std::sync::atomic::Ordering::Relaxed);
            if running_tasks == 0 {
                break;
            }
            std::thread::yield_now();
        }
    }

    pub fn drain_async(self) -> ExecutorDrain {
        ExecutorDrain {
            executor: self
        }
    }


    fn spawn_internal(&mut self, task: Box<dyn DynSpawnedTask<Infallible>>) {
        self.imp.spawn_internal(SpawnedTask::new(task));
    }

    /**
    Sets this executor as the global executor.
*/
    pub fn set_as_global_executor(&self) {
        some_executor::global_executor::set_global_executor(self.clone_box());
    }

    /**
    Sets this executor as the thread executor.
*/
    pub fn set_as_thread_executor(&self) {
        some_executor::thread_executor::set_thread_executor(self.clone_box());
    }
}

impl some_executor::SomeExecutor for Executor {
    type ExecutorNotifier = Infallible;

    fn spawn<F: Future + Send + 'static, Notifier: ObserverNotified<F::Output> + Send>(&mut self, task: Task<F, Notifier>) -> impl Observer<Value=F::Output>
    where
        Self: Sized,
        F::Output: Send
    {
        let (spawned,observer) = task.spawn(self);
        self.spawn_internal(Box::new(spawned));
        observer
    }

    fn spawn_async<'s, F: Future + Send + 'static, Notifier: ObserverNotified<F::Output> + Send>(&'s mut self, task: Task<F, Notifier>) -> impl Future<Output=impl Observer<Value=F::Output>> + Send + 's
    where
        Self: Sized,
        F::Output: Send + Unpin,
    {
        async {
            let (spawned,observer) = task.spawn(self);
            self.spawn_internal(Box::new(spawned));
            observer
        }
    }

    fn spawn_objsafe(&mut self, task: Task<Pin<Box<dyn Future<Output=Box<dyn Any + 'static + Send>> + 'static + Send>>, Box<dyn ObserverNotified<dyn Any + Send> + Send>>) -> Box<dyn Observer<Value=Box<dyn Any + Send>, Output=some_executor::observer::FinishedObservation<Box<dyn Any + Send>>> + Send>
    {
        let (spawned,observer) = task.spawn_objsafe(self);
        self.spawn_internal(Box::new(spawned));
        Box::new(observer)
    }

    fn spawn_objsafe_async<'s>(&'s mut self, task: Task<Pin<Box<dyn Future<Output=Box<dyn Any + 'static + Send>> + 'static + Send>>, Box<dyn ObserverNotified<dyn Any + Send> + Send>>) -> Box<dyn Future<Output=Box<dyn Observer<Value=Box<dyn Any + Send>, Output=some_executor::observer::FinishedObservation<Box<dyn Any + Send>>> + Send>> + 's> {
        Box::new(async {
            self.spawn_objsafe(task)
        })
    }

    fn clone_box(&self) -> Box<DynExecutor> {
        Box::new(self.clone())
    }

    fn executor_notifier(&mut self) -> Option<Self::ExecutorNotifier> {
        None
    }
}

//boilerplates









#[cfg(test)] mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use logwise::debuginternal_sync;
    use some_executor::observer::Observation;
    use some_executor::SomeExecutor;
    use some_executor::task::Configuration;
    use test_executors::async_test;
    use some_executor::observer::Observer;

    #[cfg(target_arch = "wasm32")]
    use wasm_thread as thread;
    #[cfg(not(target_arch = "wasm32"))]
    use std::thread;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);


    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn new() {
        logwise::context::Context::reset("new");
        let e = super::Executor::new("test".to_string(), 4);
        e.drain();
    }

    #[test_executors::async_test]
    async fn spawn() {
        logwise::context::Context::reset("spawn");
        let mut e = super::Executor::new("test".to_string(), 1);
        let (sender,fut) = r#continue::continuation();
        let t = some_executor::task::Task::without_notifications("test spawn".to_string(), Configuration::default(), async move {
            sender.send(1);
        });
        let _observer = e.spawn(t);
        let r = fut.await;
        assert_eq!(r,1);
    }

    #[test_executors::async_test]
    async fn poll_count() {
        logwise::context::Context::reset("poll_count");

        struct F(u32);
        impl Future for F {
            type Output = ();

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                logwise::debuginternal_sync!("poll_count is polling against {count}",count=self.0);
                if self.0 == 0 {
                    Poll::Ready(())
                } else {
                    self.get_mut().0 -= 1;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
        }
        let f = F(3);
        let mut e = super::Executor::new("poll_count".to_string(), 4);
        let task = some_executor::task::Task::without_notifications("poll_count".to_string(), Configuration::default(), f);
        let observer = e.spawn(task);
        debuginternal_sync!("drain async");
        e.drain_async().await;
        debuginternal_sync!("drain done");
        assert_eq!(observer.observe(), Observation::Ready(()));
    }

    #[async_test]
    async fn poll_outline() {
        logwise::context::Context::reset("poll_outline");
        let memory_logger = std::sync::Arc::new(logwise::InMemoryLogger::new());
        logwise::set_global_logger(memory_logger.clone());
        struct F(u32);
        impl Future for F {
            type Output = ();

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                if self.0 == 0 {
                    Poll::Ready(())
                } else {
                    let waker = cx.waker().clone();
                    self.0 -= 1;
                    thread::spawn(move || {
                        thread::sleep(std::time::Duration::from_millis(10));
                        waker.wake();
                    });
                    Poll::Pending
                }
            }

        }
        let f = F(10);
        let mut e = super::Executor::new("poll_count".to_string(), 4);
        let task = some_executor::task::Task::without_notifications("poll_count".to_string(), Configuration::default(), f);
        let observer = e.spawn(task);
        e.drain_async().await;
        assert_eq!(observer.observe(), Observation::Ready(()));
    }
}
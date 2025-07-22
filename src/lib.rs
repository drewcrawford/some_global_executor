use std::any::Any;
use std::convert::Infallible;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::{Arc};
use std::sync::atomic::AtomicUsize;
use atomic_waker::AtomicWaker;
use crossbeam_channel::{Receiver, Sender};
use logwise::{debuginternal_sync, info_sync};
use some_executor::DynExecutor;
use some_executor::observer::{Observer, ObserverNotified};
use some_executor::task::{DynSpawnedTask, Task};
use crate::threadpool::{ThreadBuilder, ThreadFn, ThreadMessage, Threadpool};
use some_executor::SomeExecutor;

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
        self.executor.inner.drain_notify.waker.register(cx.waker());
        let running_tasks = self.executor.inner.drain_notify.running_tasks.load(std::sync::atomic::Ordering::Relaxed);
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

mod threadpool;
mod waker;
mod sys;

#[derive(Debug)]
struct Builder {
    receiver: crossbeam_channel::Receiver<SpawnedTask>,
    queue_task: Sender<SpawnedTask>,
    drain_notify: Arc<DrainNotify>,
}
struct Thread {
    imp: sys::Thread,
}
impl ThreadBuilder for Builder {
    type ThreadFn = Thread;
    fn build(&mut self) -> Self::ThreadFn {
        Thread {
            imp: sys::Thread::new(
                self.receiver.clone(),
                self.queue_task.clone(),
                self.drain_notify.clone(),
            ),
        }
    }
}
impl ThreadFn for Thread {
    fn run(self, receiver: Receiver<ThreadMessage>) {
        self.imp.run(receiver);
    }
}

#[derive(Debug)]
struct Inner {
    _threadpool: Threadpool<Builder>,
    sender: Sender<SpawnedTask>,
    drain_notify: Arc<DrainNotify>,
}

#[derive(Debug,Clone)]
pub struct Executor {
    inner: Arc<Inner>,
}

struct SpawnedTask {
    imp: sys::SpawnedTask,
}


impl SpawnedTask {
    fn new(task: Box<dyn DynSpawnedTask<Infallible>>) -> Self {
        Self {
            imp: sys::SpawnedTask::new(task),
        }
    }

    fn run(self, task_sender: Sender<SpawnedTask>, drain_notify: &DrainNotify) {
        self.imp.run(task_sender, drain_notify);
    }
}


impl Executor {
    pub fn new(name: String, size: usize) -> Self {
        info_sync!("Executor::new name={name} size={size}",name=(&name as &str),size=(size as u64));
        let (sender,receiver) = crossbeam_channel::unbounded();
        let drain_notify = Arc::new(DrainNotify {
            running_tasks: AtomicUsize::new(0),
            waker: AtomicWaker::new(),
        });
        let builder = Builder {
            receiver,
            queue_task: sender.clone(),
            drain_notify: drain_notify.clone()
        };
        let threadpool = Threadpool::new(name, size, builder);
        let inner = Arc::new(Inner {
            drain_notify: drain_notify,
            _threadpool: threadpool,
            sender
        });
        Self {
            inner,
        }
    }

    pub fn new_default() -> Self {
        Self::new("default".to_string(), num_cpus::get())
    }




    pub fn drain(self) {
        let _interval = logwise::perfwarn_begin!("Executor::drain busyloop");
        loop {
            let running_tasks = self.inner.drain_notify.running_tasks.load(std::sync::atomic::Ordering::Relaxed);
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
        self.inner.drain_notify.running_tasks.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
        let spawned_task = SpawnedTask::new(task);
        logwise::warn_sync!("Sending task {task}",task=logwise::privacy::LogIt(spawned_task.imp.task_id()));
        self.inner.sender.send(spawned_task).unwrap();
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

impl PartialEq for Executor {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for Executor {}

impl Hash for Executor {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.inner).hash(state)
    }
}




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
        struct F(u32);
        impl Future for F {
            type Output = ();

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                if self.0 == 0 {
                    Poll::Ready(())
                } else {
                    let waker = cx.waker().clone();
                    self.0 -= 1;
                    crate::threadpool::sys::spawn(move || {
                        crate::threadpool::sys::sleep(std::time::Duration::from_millis(10));
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
use std::any::Any;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use crossbeam_channel::Receiver;
use some_executor::DynExecutor;
use some_executor::observer::{ExecutorNotified, Observer, ObserverNotified};
use some_executor::task::{DynSpawnedTask, Task};
use crate::threadpool::{ThreadBuilder, ThreadFn, ThreadMessage, Threadpool};

mod threadpool;

struct Builder {

}
struct Thread {

}
impl ThreadBuilder for Builder {
    type ThreadFn = Thread;
    fn build(&mut self) -> Self::ThreadFn {
        Thread {

        }
    }
}
impl ThreadFn for Thread {
    fn run(self, receiver: Receiver<ThreadMessage>) {
        loop {
            match receiver.recv() {
                Ok(ThreadMessage::Shutdown) => {
                    break;
                }
                Err(_) => {
                    break;
                }
            }
        }
    }
}

struct Inner {
    threadpool: Threadpool<Builder>,
}

#[derive(Clone)]
pub struct Executor {
    inner: Arc<Inner>,
}

struct SpawnedTask {
    task: Box<dyn DynSpawnedTask<Executor>>,
}


impl Executor {
    pub fn new(name: String, size: usize) -> Self {
        let builder = Builder {};
        let threadpool = Threadpool::new(name, size, builder);
        let inner = Arc::new(Inner {
            threadpool
        });
        Self {
            inner
        }
    }

    pub fn new_default() -> Self {
        Self::new("default".to_string(), num_cpus::get())
    }

    pub fn join(self) {
        self.inner.threadpool.join();
    }

    fn spawn(&mut self, task: Box<dyn DynSpawnedTask<Self>>) {
        todo!()
    }
}

impl some_executor::SomeExecutor for Executor {
    type ExecutorNotifier = Infallible;

    fn spawn<F: Future + Send + 'static, Notifier: ObserverNotified<F::Output> + Send>(&mut self, task: Task<F, Notifier>) -> Observer<F::Output, Self::ExecutorNotifier>
    where
        Self: Sized,
        F::Output: Send
    {
        let (spawned,observer) = task.spawn(self);
        self.spawn(Box::new(spawned));
        observer
    }

    fn spawn_async<F: Future + Send + 'static, Notifier: ObserverNotified<F::Output> + Send>(&mut self, task: Task<F, Notifier>) -> impl Future<Output=Observer<F::Output, Self::ExecutorNotifier>> + Send + 'static
    where
        Self: Sized,
        F::Output: Send
    {
        let (spawned,observer) = task.spawn(self);
        self.spawn(Box::new(spawned));

        async {
            observer
        }
    }

    fn spawn_objsafe(&mut self, task: Task<Pin<Box<dyn Future<Output=Box<dyn Any + 'static + Send>> + 'static + Send>>, Box<dyn ObserverNotified<dyn Any + Send> + Send>>) -> Observer<Box<dyn Any + 'static + Send>, Box<dyn ExecutorNotified + 'static + Send>> {
        let (spawned,observer) = task.spawn_objsafe(self);
        self.spawn(Box::new(spawned));
        observer
    }

    fn clone_box(&self) -> Box<DynExecutor> {
        Box::new(self.clone())
    }

    fn executor_notifier(&mut self) -> Option<Self::ExecutorNotifier> {
        None
    }
}



#[cfg(test)] mod tests {
    #[test] fn new() {
        let e = super::Executor::new("test".to_string(), 4);
        e.join();
    }
}
use std::any::Any;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Waker;
use crossbeam_channel::{select_biased, Receiver, Sender};
use some_executor::DynExecutor;
use some_executor::observer::{ExecutorNotified, Observer, ObserverNotified};
use some_executor::task::{DynSpawnedTask, Task};
use crate::threadpool::{ThreadBuilder, ThreadFn, ThreadMessage, Threadpool};
use crate::waker::WakeInternal;

mod threadpool;
mod waker;

struct Builder {
    receiver: crossbeam_channel::Receiver<SpawnedTask>,
    queue_task: Sender<SpawnedTask>,
}
struct Thread {
    receiver: crossbeam_channel::Receiver<SpawnedTask>,
    queue_task: Sender<SpawnedTask>,
}
impl ThreadBuilder for Builder {
    type ThreadFn = Thread;
    fn build(&mut self) -> Self::ThreadFn {
        Thread {
            receiver: self.receiver.clone(),
            queue_task: self.queue_task.clone(),
        }
    }
}
impl ThreadFn for Thread {
    fn run(self, receiver: Receiver<ThreadMessage>) {
        loop {
            select_biased!(
                recv(self.receiver) -> task => {
                    match task {
                        Ok(mut task) => {
                            task.run(self.queue_task.clone());
                        }
                        Err(_) => {
                            break;
                        }
                    }
                },
                recv(receiver) -> message => {
                    match message {
                        Ok(ThreadMessage::Shutdown) => {
                            break;
                        }
                        Err(_) => {
                            break;
                        }
                    }
                }
            );
        }
    }
}

struct Inner {
    threadpool: Threadpool<Builder>,
    sender: Sender<SpawnedTask>
}

#[derive(Clone)]
pub struct Executor {
    inner: Arc<Inner>,
}

struct SpawnedTask {
    task: Pin<Box<dyn DynSpawnedTask<Infallible>>>,
    waker: Waker,
    wake_internal: Arc<WakeInternal>
}


impl SpawnedTask {
    fn new(task: Box<dyn DynSpawnedTask<Infallible>>) -> Self {
        let (waker,wake_internal) = crate::waker::task_waker();
        let task = Box::into_pin(task);
        Self {
            task,
            waker,
            wake_internal,
        }
    }

    fn run(mut self, task_sender: Sender<SpawnedTask>) {
        let mut context = std::task::Context::from_waker(&self.waker);
        self.wake_internal.reset();
        DynSpawnedTask::poll(self.task.as_mut(), &mut context,None);
        let move_wake_inernal = self.wake_internal.clone();
        move_wake_inernal.check_wake(|| {
            task_sender.send(self).unwrap()
        });
    }
}


impl Executor {
    pub fn new(name: String, size: usize) -> Self {
        let (sender,receiver) = crossbeam_channel::unbounded();
        let builder = Builder {
            receiver,
            queue_task: sender.clone(),
        };
        let threadpool = Threadpool::new(name, size, builder);
        let inner = Arc::new(Inner {
            threadpool,
            sender
        });
        Self {
            inner,
        }
    }

    pub fn new_default() -> Self {
        Self::new("default".to_string(), num_cpus::get())
    }

    pub fn join(self) {
        self.inner.threadpool.join();
    }

    fn spawn_internal(&mut self, task: Box<dyn DynSpawnedTask<Infallible>>) {
        let spawned_task = SpawnedTask::new(task);
        self.inner.sender.send(spawned_task).unwrap();
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
        self.spawn_internal(Box::new(spawned));
        observer
    }

    fn spawn_async<F: Future + Send + 'static, Notifier: ObserverNotified<F::Output> + Send>(&mut self, task: Task<F, Notifier>) -> impl Future<Output=Observer<F::Output, Self::ExecutorNotifier>> + Send + 'static
    where
        Self: Sized,
        F::Output: Send
    {
        let (spawned,observer) = task.spawn(self);
        self.spawn_internal(Box::new(spawned));

        async {
            observer
        }
    }

    fn spawn_objsafe(&mut self, task: Task<Pin<Box<dyn Future<Output=Box<dyn Any + 'static + Send>> + 'static + Send>>, Box<dyn ObserverNotified<dyn Any + Send> + Send>>) -> Observer<Box<dyn Any + 'static + Send>, Box<dyn ExecutorNotified + 'static + Send>> {
        let (spawned,observer) = task.spawn_objsafe(self);
        self.spawn_internal(Box::new(spawned));
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
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use some_executor::observer::Observation;
    use some_executor::SomeExecutor;
    use some_executor::task::Configuration;

    #[test] fn new() {
        let e = super::Executor::new("test".to_string(), 4);
        e.join();
    }

    #[test] fn spawn() {
        let mut e = super::Executor::new("test".to_string(), 4);
        let (sender,receiver) = std::sync::mpsc::channel();
        let t = some_executor::task::Task::without_notifications("test spawn".to_string(),async move {
            sender.send(1).unwrap();
        }, Configuration::default());
        let observer = e.spawn(t);
        let r = receiver.recv().unwrap();
        assert_eq!(r,1);
    }

    #[test] fn poll_count() {
        struct F(u32);
        impl Future for F {
            type Output = ();

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                if self.0 == 0 {
                    Poll::Ready(())
                } else {
                    self.get_mut().0 -= 1;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
        }
        let f = F(10);
        let mut e = super::Executor::new("poll_count".to_string(), 4);
        let task = some_executor::task::Task::without_notifications("poll_count".to_string(),f, Configuration::default());
        let observer = e.spawn(task);
        e.join();
        assert_eq!(observer.observe(), Observation::Ready(()));

    }
}
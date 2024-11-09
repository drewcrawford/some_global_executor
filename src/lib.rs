use std::any::Any;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc};
use std::sync::atomic::AtomicUsize;
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
    running_tasks: Arc<AtomicUsize>,
}
struct Thread {
    receiver: crossbeam_channel::Receiver<SpawnedTask>,
    queue_task: Sender<SpawnedTask>,
    running_tasks: Arc<AtomicUsize>,
}
impl ThreadBuilder for Builder {
    type ThreadFn = Thread;
    fn build(&mut self) -> Self::ThreadFn {
        Thread {
            receiver: self.receiver.clone(),
            queue_task: self.queue_task.clone(),
            running_tasks: self.running_tasks.clone(),
        }
    }
}
impl ThreadFn for Thread {
    fn run(self, receiver: Receiver<ThreadMessage>) {
        loop {
            select_biased!(
                recv(self.receiver) -> task => {
                    match task {
                        Ok(task) => {
                            let mut _interval = None;
                            if task.task.priority() < some_executor::Priority::UserInteractive {
                                _interval = Some(logwise::perfwarn_begin!("Threadpool::run does not sort tasks"));
                            }
                            task.run(self.queue_task.clone(), &self.running_tasks);
                            _interval = None;
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
    _threadpool: Threadpool<Builder>,
    sender: Sender<SpawnedTask>,
    running_tasks: Arc<AtomicUsize>,
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

    fn run(mut self, task_sender: Sender<SpawnedTask>, running_tasks: &AtomicUsize) {
        let mut context = std::task::Context::from_waker(&self.waker);
        self.wake_internal.reset();
        let r = DynSpawnedTask::poll(self.task.as_mut(), &mut context,None);
        match r {
            std::task::Poll::Ready(_) => {
                running_tasks.fetch_sub(1,std::sync::atomic::Ordering::Relaxed);
            },
            std::task::Poll::Pending => {
                let move_wake_inernal = self.wake_internal.clone();
                move_wake_inernal.check_wake(move || {
                    task_sender.send(self).unwrap()
                });
            }
        }
    }
}


impl Executor {
    pub fn new(name: String, size: usize) -> Self {
        let (sender,receiver) = crossbeam_channel::unbounded();
        let running_tasks = Arc::new(AtomicUsize::new(0));
        let builder = Builder {
            receiver,
            queue_task: sender.clone(),
            running_tasks: running_tasks.clone()
        };
        let threadpool = Threadpool::new(name, size, builder);
        let inner = Arc::new(Inner {
            running_tasks,
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
            let running_tasks = self.inner.running_tasks.load(std::sync::atomic::Ordering::Relaxed);
            if running_tasks == 0 {
                break;
            }
            std::thread::yield_now();
        }
    }


    fn spawn_internal(&mut self, task: Box<dyn DynSpawnedTask<Infallible>>) {
        self.inner.running_tasks.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
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
        e.drain();
    }

    #[test] fn spawn() {
        let mut e = super::Executor::new("test".to_string(), 4);
        let (sender,receiver) = std::sync::mpsc::channel();
        let t = some_executor::task::Task::without_notifications("test spawn".to_string(),async move {
            sender.send(1).unwrap();
        }, Configuration::default());
        let _observer = e.spawn(t);
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
        e.drain();
        assert_eq!(observer.observe(), Observation::Ready(()));

    }

    #[test] fn poll_outline() {
        struct F(u32);
        impl Future for F {
            type Output = ();

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                if self.0 == 0 {
                    Poll::Ready(())
                } else {
                    let waker = cx.waker().clone();
                    self.0 -= 1;
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        waker.wake();
                    });
                    Poll::Pending
                }
            }

        }
        let f = F(10);
        let mut e = super::Executor::new("poll_count".to_string(), 4);
        let task = some_executor::task::Task::without_notifications("poll_count".to_string(),f, Configuration::default());
        let observer = e.spawn(task);
        e.drain();
        assert_eq!(observer.observe(), Observation::Ready(()));
    }
}
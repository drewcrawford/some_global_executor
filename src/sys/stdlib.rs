use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc};
use std::task::Waker;
use crossbeam_channel::{select_biased, Receiver, Sender};
use some_executor::task::DynSpawnedTask;
use crate::waker::WakeInternal;
use crate::threadpool::{ThreadBuilder, ThreadFn, ThreadMessage};

#[derive(Debug)]
pub struct SpawnedTask {
    task: Pin<Box<dyn DynSpawnedTask<Infallible>>>,
    waker: Waker,
    wake_internal: Arc<WakeInternal>
}

impl SpawnedTask {
    pub fn new(task: Box<dyn DynSpawnedTask<Infallible>>) -> Self {
        let (waker, wake_internal) = crate::waker::task_waker();
        let task = Box::into_pin(task);
        Self {
            task,
            waker,
            wake_internal,
        }
    }

    pub(crate) fn run(mut self, task_sender: Sender<crate::SpawnedTask>, drain_notify: &crate::DrainNotify) {
        let mut context = std::task::Context::from_waker(&self.waker);
        self.wake_internal.reset();
        logwise::debuginternal_sync!("Polling task {task}",task=logwise::privacy::LogIt(self.task.task_id()));
        let r = DynSpawnedTask::poll(self.task.as_mut(), &mut context, None);
        match r {
            std::task::Poll::Ready(_) => {
                let old = drain_notify.running_tasks.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                logwise::debuginternal_sync!("Task {task} finished, running_tasks={running_tasks}",task=logwise::privacy::LogIt(self.task.task_id()),running_tasks=(old - 1));
                if old == 1 {
                    drain_notify.waker.wake();
                }
            },
            std::task::Poll::Pending => {
                let move_wake_internal = self.wake_internal.clone();
                move_wake_internal.check_wake(move || {
                    let main_spawned_task = crate::SpawnedTask { imp: self };
                    task_sender.send(main_spawned_task).unwrap()
                });
            }
        }
    }

    pub fn task_id(&self) -> impl std::fmt::Debug {
        self.task.task_id()
    }

    pub(crate) fn priority(&self) -> some_executor::Priority {
        self.task.priority()
    }
}

#[derive(Debug)]
pub struct Thread {
    receiver: crossbeam_channel::Receiver<crate::SpawnedTask>,
    queue_task: Sender<crate::SpawnedTask>,
    drain_notify: Arc<crate::DrainNotify>,
}

impl Thread {
    pub fn new(
        receiver: crossbeam_channel::Receiver<crate::SpawnedTask>,
        queue_task: Sender<crate::SpawnedTask>,
        drain_notify: Arc<crate::DrainNotify>,
    ) -> Self {
        Self {
            receiver,
            queue_task,
            drain_notify,
        }
    }

    pub fn run(self, receiver: Receiver<ThreadMessage>) {
        logwise::debuginternal_sync!("Thread::run");
        loop {
            select_biased!(
                recv(self.receiver) -> task => {
                    match task {
                        Ok(task) => {
                            let mut _interval = None;
                            if task.imp.priority() < some_executor::Priority::UserInteractive {
                                _interval = Some(logwise::perfwarn_begin!("Threadpool::run does not sort tasks"));
                            }
                            task.imp.run(self.queue_task.clone(), &self.drain_notify);
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

#[derive(Debug)]
pub struct Builder {
    receiver: crossbeam_channel::Receiver<crate::SpawnedTask>,
    queue_task: Sender<crate::SpawnedTask>,
    drain_notify: Arc<crate::DrainNotify>,
}

impl Builder {
    pub fn new(receiver: crossbeam_channel::Receiver<crate::SpawnedTask>, queue_task: Sender<crate::SpawnedTask>, drain_notify: Arc<crate::DrainNotify>) -> Self {
        Self {
            receiver,
            queue_task,
            drain_notify,
        }
    }
}

pub struct ThreadImpl {
    pub(crate) imp: Thread,
}

impl ThreadBuilder for Builder {
    type ThreadFn = ThreadImpl;
    fn build(&mut self) -> Self::ThreadFn {
        ThreadImpl {
            imp: Thread::new(
                self.receiver.clone(),
                self.queue_task.clone(),
                self.drain_notify.clone(),
            ),
        }
    }
}

impl ThreadFn for ThreadImpl {
    fn run(self, receiver: Receiver<ThreadMessage>) {
        self.imp.run(receiver);
    }
}

#[derive(Debug)]
pub struct Threadpool<B> {
    vec: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>,
    requested_threads: usize,
    sender: Sender<crate::threadpool::ThreadMessage>,
    receiver: Receiver<crate::threadpool::ThreadMessage>,
    name: String,
    thread_builder: B,
}

impl<B> Threadpool<B> {
    pub fn new(name: String, size: usize, mut thread_builder: B) -> Self
    where B: crate::threadpool::ThreadBuilder, {
        let mut vec = Vec::with_capacity(size);
        let (sender,receiver) = crossbeam_channel::unbounded();
        for t in 0..size {
            let receiver = receiver.clone();
            let thread_fn = thread_builder.build();
            let handle = Self::build_thread(&name, t, receiver, thread_fn);
            vec.push(handle);
        }
        let vec = std::sync::Mutex::new(vec);
        Threadpool { vec, sender, receiver,name,thread_builder, requested_threads: size }
    }

    fn build_thread<T: crate::threadpool::ThreadFn>(name: &str, thread_no: usize, receiver: Receiver<crate::threadpool::ThreadMessage>,thread_fn: T) -> std::thread::JoinHandle<()> {
        let name = format!("some_global_executor {}-{}", name, thread_no);
        std::thread::Builder::new()
            .name(name)
            .spawn(move || {
                let c = logwise::context::Context::new_task(None, "some_global_executor threadpool");
                c.set_current();
                thread_fn.run(receiver);
            })
            .unwrap()
    }

    pub async fn resize(&mut self, size: usize)
    where B: crate::threadpool::ThreadBuilder {
        let mut lock = self.vec.lock().unwrap();
        let old_size = self.requested_threads;
        if size > old_size {
            for t in old_size..size {
                let receiver = self.receiver.clone();
                let handle = Self::build_thread(&self.name, t, receiver,self.thread_builder.build());
                lock.push(handle);
            }
        } else {
            for _ in size..old_size {
                self.sender.send(crate::threadpool::ThreadMessage::Shutdown).unwrap();
            }
            //gc
            lock.retain(|handle| !handle.is_finished());

        }
    }
}
// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::DrainNotify;
use crate::waker::WakeInternal;
use crossbeam_channel::{Receiver, Sender, select_biased};
use logwise::info_sync;
use some_executor::task::DynSpawnedTask;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Waker;

#[derive(Debug)]
pub struct SpawnedTask {
    task: Pin<Box<dyn DynSpawnedTask<Infallible>>>,
    waker: Waker,
    wake_internal: Arc<WakeInternal>,
}

pub fn default_threadpool_size() -> usize {
    num_cpus::get()
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

    pub(crate) fn run(
        mut self,
        task_sender: Sender<crate::SpawnedTask>,
        drain_notify: &crate::DrainNotify,
    ) {
        let mut context = std::task::Context::from_waker(&self.waker);
        self.wake_internal.reset();
        logwise::debuginternal_sync!(
            "Polling task {task}",
            task = logwise::privacy::LogIt(self.task.task_id())
        );
        let r = DynSpawnedTask::poll(self.task.as_mut(), &mut context, None);
        match r {
            std::task::Poll::Ready(_) => {
                let old = drain_notify
                    .running_tasks
                    .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                logwise::debuginternal_sync!(
                    "Task {task} finished, running_tasks={running_tasks}",
                    task = logwise::privacy::LogIt(self.task.task_id()),
                    running_tasks = (old - 1)
                );
                if old == 1 {
                    drain_notify.waker.wake();
                }
            }
            std::task::Poll::Pending => {
                let move_wake_internal = self.wake_internal.clone();
                move_wake_internal.check_wake(move || {
                    let main_spawned_task = crate::SpawnedTask { imp: self };
                    task_sender.send(main_spawned_task).unwrap()
                });
            }
        }
    }

    fn task_id(&self) -> impl std::fmt::Debug {
        self.task.task_id()
    }

    pub(crate) fn priority(&self) -> some_executor::Priority {
        self.task.priority()
    }
}

enum ThreadMessage {
    Shutdown,
}
#[derive(Debug)]
struct Thread {
    receiver: crossbeam_channel::Receiver<crate::SpawnedTask>,
    thread_receiver: crossbeam_channel::Receiver<ThreadMessage>,
    queue_task: Sender<crate::SpawnedTask>,
    drain_notify: Arc<crate::DrainNotify>,
}

impl Thread {
    fn new(
        receiver: crossbeam_channel::Receiver<crate::SpawnedTask>,
        queue_task: Sender<crate::SpawnedTask>,
        drain_notify: Arc<crate::DrainNotify>,
        thread_receiver: crossbeam_channel::Receiver<ThreadMessage>,
    ) -> Self {
        Self {
            receiver,
            queue_task,
            drain_notify,
            thread_receiver,
        }
    }
}

impl Thread {
    fn run(self) {
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
                recv(self.thread_receiver) -> message => {
                    match message {
                        Ok(ThreadMessage::Shutdown) => {
                            logwise::debuginternal_sync!("Received shutdown message, exiting thread");
                            break;
                        }
                        Err(_) => {
                           logwise::debuginternal_sync!("Threadpool shutting down");
                            break;
                        }
                    }
                }
            );
        }
        logwise::warn_sync!("thread shutdown");
    }
}

#[derive(Debug)]
struct Threadpool {
    vec: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>,
    requested_threads: usize,
    thread_sender: Sender<ThreadMessage>,
    thread_receiver: Receiver<ThreadMessage>,
    task_receiver: crossbeam_channel::Receiver<crate::SpawnedTask>,
    task_sender: Sender<crate::SpawnedTask>,
    drain_notify: Arc<crate::DrainNotify>,
    name: String,
}

impl Threadpool {
    fn new(name: String, size: usize) -> Self {
        let mut vec = Vec::with_capacity(size);
        let (task_sender, task_receiver) = crossbeam_channel::unbounded();
        let (thread_sender, thread_receiver) = crossbeam_channel::unbounded();
        let drain_notify = Arc::new(crate::DrainNotify::new());
        for t in 0..size {
            let task_receiver = task_receiver.clone();
            let task_sender = task_sender.clone();
            let drain_notify = drain_notify.clone();
            let thread_receiver = thread_receiver.clone();
            let thread = Thread::new(task_receiver, task_sender, drain_notify, thread_receiver);
            let handle = Self::build_thread(&name, t, thread);
            vec.push(handle);
        }
        let vec = std::sync::Mutex::new(vec);
        Threadpool {
            vec,
            requested_threads: size,
            thread_sender,
            thread_receiver,
            task_receiver,
            task_sender,
            drain_notify,
            name,
        }
    }

    fn build_thread(name: &str, thread_no: usize, thread: Thread) -> std::thread::JoinHandle<()> {
        let name = format!("some_global_executor {}-{}", name, thread_no);
        std::thread::Builder::new()
            .name(name)
            .spawn(move || {
                let c = logwise::context::Context::new_task(
                    None,
                    "some_global_executor threadpool".to_string(),
                );
                c.set_current();
                thread.run();
            })
            .unwrap()
    }

    fn resize(&self, size: usize) {
        let mut lock = self.vec.lock().unwrap();
        let old_size = self.requested_threads;
        if size > old_size {
            for t in old_size..size {
                let task_receiver = self.task_receiver.clone();
                let task_sender = self.task_sender.clone();
                let drain_notify = self.drain_notify.clone();
                let thread_receiver = self.thread_receiver.clone();
                let thread = Thread::new(task_receiver, task_sender, drain_notify, thread_receiver);

                let handle = Self::build_thread(&self.name, t, thread);
                lock.push(handle);
            }
        } else {
            for _ in size..old_size {
                self.thread_sender.send(ThreadMessage::Shutdown).unwrap();
            }
            //gc
            lock.retain(|handle| !handle.is_finished());
        }
    }
}

#[derive(Debug)]
struct Inner {
    threadpool: Threadpool,
}

#[derive(Debug, Clone)]
pub struct Executor {
    inner: Arc<Inner>,
}

impl PartialEq for Executor {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for Executor {}
impl std::hash::Hash for Executor {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.inner).hash(state);
    }
}

impl Executor {
    pub fn new(name: String, size: usize) -> Self {
        info_sync!(
            "Executor::new name={name} size={size}",
            name = (&name as &str),
            size = (size as u64)
        );
        let threadpool = Threadpool::new(name, size);
        let inner = Arc::new(Inner { threadpool });
        Self { inner }
    }

    pub fn drain_notify(&self) -> &Arc<DrainNotify> {
        &self.inner.threadpool.drain_notify
    }
    pub fn spawn_internal(&self, spawned_task: crate::SpawnedTask) {
        self.drain_notify()
            .running_tasks
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        logwise::debuginternal_sync!(
            "Sending task {task}",
            task = logwise::privacy::LogIt(spawned_task.imp.task_id())
        );
        self.inner
            .threadpool
            .task_sender
            .send(spawned_task)
            .unwrap();
    }
    pub fn resize(&mut self, size: usize) {
        logwise::debuginternal_sync!("Executor::resize size={size}", size = (size as u64));
        self.inner.threadpool.resize(size);
    }
}

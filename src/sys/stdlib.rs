// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::DrainNotify;
use crate::waker::WakeInternal;
use crossbeam_channel::{Receiver, Sender, select_biased};
use logwise::info_sync;
use some_executor::observer::{Observer, ObserverNotified};
use some_executor::static_support::OwnedSomeStaticExecutorErasingNotifier;
use some_executor::task::{DynSpawnedTask, Task};
use some_executor::{BoxedStaticObserver, BoxedStaticObserverFuture, DynStaticExecutor, ObjSafeStaticTask, Priority, SomeStaticExecutor};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::convert::Infallible;
use std::future::Future;
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

/// A spawned static task that does not require Send.
/// These tasks run on the same thread where they were spawned.
struct SpawnedStaticTask {
    id: u64,
    future: Pin<Box<dyn Future<Output = ()> + 'static>>,
    waker: Waker,
    wake_internal: Arc<WakeInternal>,
    drain_notify: Arc<DrainNotify>,
    pub(crate) priority: some_executor::Priority,

}

impl std::fmt::Debug for SpawnedStaticTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnedStaticTask")
            .field("id", &self.id)
            .field("wake_internal", &self.wake_internal)
            .finish_non_exhaustive()
    }
}

impl SpawnedStaticTask {
    fn new<F>(future: F, drain_notify: Arc<DrainNotify>, priority: Priority) -> Self
    where
        F: Future<Output = ()> + 'static,
    {
        let (waker, wake_internal) = crate::waker::task_waker();
        let id = STATIC_TASK_ID_COUNTER.with(|counter| {
            let mut c = counter.borrow_mut();
            let id = *c;
            *c = c.wrapping_add(1);
            id
        });
        Self {
            id,
            future: Box::pin(future),
            waker,
            wake_internal,
            drain_notify,
            priority

        }
    }

    fn run(mut self) {
        let mut context = std::task::Context::from_waker(&self.waker);
        self.wake_internal.reset();
        logwise::debuginternal_sync!("Polling static task {id}", id = self.id);
        let r = self.future.as_mut().poll(&mut context);
        match r {
            std::task::Poll::Ready(()) => {
                let old = self
                    .drain_notify
                    .running_tasks
                    .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                logwise::debuginternal_sync!(
                    "Static task {id} finished, running_tasks={running_tasks}",
                    id = self.id,
                    running_tasks = (old - 1)
                );
                if old == 1 {
                    self.drain_notify.waker.wake();
                }
            }
            std::task::Poll::Pending => {
                // Store the task in pending map and set up wake callback
                let task_id = self.id;
                let move_wake_internal = self.wake_internal.clone();

                // Capture the sender before storing task (must be done on this thread)
                let sender = STATIC_NOTIFY_SENDER.with(|sender| {
                    sender.borrow().clone()
                });

                // Store in pending tasks map
                STATIC_PENDING_TASKS.with(|pending| {
                    pending.borrow_mut().insert(task_id, self);
                });

                // Set up wake callback to send task ID (which is Send)
                // The sender is captured so this works from any thread
                move_wake_internal.check_wake(move || {
                    if let Some(sender) = sender {
                        let _ = sender.send(task_id);
                    }
                });
            }
        }
    }
}

// Thread-local storage for static tasks
thread_local! {
    /// Queue of ready-to-run static tasks
    static STATIC_TASKS: RefCell<VecDeque<SpawnedStaticTask>> = RefCell::new(VecDeque::new());
    /// Sender to notify the thread of new tasks
    static STATIC_NOTIFY_SENDER: RefCell<Option<Sender<u64>>> = RefCell::new(None);
    /// Reference to the drain notify for task completion
    static STATIC_DRAIN_NOTIFY: RefCell<Option<Arc<DrainNotify>>> = RefCell::new(None);
    /// Map of pending (awaiting wake) static tasks, indexed by task ID
    static STATIC_PENDING_TASKS: RefCell<std::collections::HashMap<u64, SpawnedStaticTask>> = RefCell::new(std::collections::HashMap::new());
    /// Counter for generating unique task IDs (starts at 1, since 0 is the "new task" indicator)
    static STATIC_TASK_ID_COUNTER: RefCell<u64> = const { RefCell::new(1) };
}

/// A static executor for running non-Send tasks on the current thread.
///
/// This executor is designed to be used within threadpool workers, where
/// each worker has its own instance. Tasks spawned through this executor
/// will run on the same thread where they were spawned.
///
/// # Performance Warning
///
/// This executor does not sort tasks by priority. High-priority static tasks
/// may be delayed by lower-priority tasks that were spawned earlier.
#[derive(Clone, Debug)]
pub struct StaticExecutor {
    drain_notify: Arc<DrainNotify>,
}

impl StaticExecutor {
    /// Creates a new static executor that shares the given drain notify.
    fn new(drain_notify: Arc<DrainNotify>) -> Self {
        StaticExecutor { drain_notify }
    }

    /// Spawns a future on the current thread's static task queue.
    fn spawn<F>(&self, fut: F, priority: Priority)
    where
        F: Future<Output = ()> + 'static,
    {
        let static_task = SpawnedStaticTask::new(fut, self.drain_notify.clone(), priority);

        // Increment running tasks count
        self.drain_notify
            .running_tasks
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Add to thread-local queue
        STATIC_TASKS.with(|tasks| {
            tasks.borrow_mut().push_back(static_task);
        });

        // Notify the thread (0 = new task indicator)
        STATIC_NOTIFY_SENDER.with(|sender| {
            if let Some(sender) = sender.borrow().as_ref() {
                let _ = sender.send(0);
            }
        });
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
        let priority = task.priority();
        let (spawned, observer) = task.spawn_static(self);
        // Convert to future and spawn it
        self.spawn(async {
            spawned.into_future().await;
        }, priority);
        observer
    }

    async fn spawn_static_async<F, Notifier: ObserverNotified<F::Output>>(
        &mut self,
        task: Task<F, Notifier>,
    ) -> impl Observer<Value = F::Output>
    where
        Self: Sized,
        F: Future + 'static,
        F::Output: 'static + Unpin,
    {
        self.spawn_static(task)
    }

    fn spawn_static_objsafe(&mut self, task: ObjSafeStaticTask) -> BoxedStaticObserver {
        Box::new(self.spawn_static(task))
    }

    fn spawn_static_objsafe_async<'s>(
        &'s mut self,
        task: ObjSafeStaticTask,
    ) -> BoxedStaticObserverFuture<'s> {
        #[allow(clippy::async_yields_async)]
        Box::new(async { self.spawn_static_objsafe(task) })
    }

    fn clone_box(&self) -> Box<DynStaticExecutor> {
        let adapt = OwnedSomeStaticExecutorErasingNotifier::new(self.clone());
        Box::new(adapt)
    }

    fn executor_notifier(&mut self) -> Option<Self::ExecutorNotifier> {
        None
    }
}

enum ThreadMessage {
    Shutdown,
}
#[derive(Debug)]
struct Thread {
    receiver: crossbeam_channel::Receiver<crate::SpawnedTask>,
    thread_receiver: crossbeam_channel::Receiver<ThreadMessage>,
    static_receiver: crossbeam_channel::Receiver<u64>,
    static_sender: Sender<u64>,
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
        let (static_sender, static_receiver) = crossbeam_channel::unbounded();
        Self {
            receiver,
            queue_task,
            drain_notify,
            thread_receiver,
            static_receiver,
            static_sender,
        }
    }
}

impl Thread {
    fn run(self) {
        logwise::debuginternal_sync!("Thread::run");

        // Install thread-local storage for static tasks
        STATIC_NOTIFY_SENDER.with(|sender| {
            *sender.borrow_mut() = Some(self.static_sender.clone());
        });
        STATIC_DRAIN_NOTIFY.with(|notify| {
            *notify.borrow_mut() = Some(self.drain_notify.clone());
        });

        // Create and install the static executor as the thread-local default
        let static_executor = StaticExecutor::new(self.drain_notify.clone());
        let boxed_executor: Box<dyn SomeStaticExecutor<ExecutorNotifier = Infallible>> =
            Box::new(static_executor);
        some_executor::thread_executor::set_thread_static_executor_adapting_notifier(boxed_executor);

        loop {
            select_biased!(
                // Check for static task notifications first (they're local and should be responsive)
                recv(self.static_receiver) -> task_id => {
                    if let Ok(task_id) = task_id {
                        // If task_id is non-zero, it's a woken task - retrieve from pending
                        if task_id != 0 {
                            let task = STATIC_PENDING_TASKS.with(|pending| {
                                pending.borrow_mut().remove(&task_id)
                            });
                            if let Some(task) = task {
                                // Add woken task to the ready queue
                                STATIC_TASKS.with(|tasks| {
                                    tasks.borrow_mut().push_back(task);
                                });
                            }
                        }
                    }
                    // Process all ready static tasks
                    loop {
                        let task = STATIC_TASKS.with(|tasks| {
                            tasks.borrow_mut().pop_front()
                        });
                        match task {
                            Some(task) => {
                                let mut _interval = None;
                                if task.priority < some_executor::Priority::UserInteractive {
                                    _interval = Some(logwise::perfwarn_begin_if!(logwise::Duration::from_millis(5), "StaticExecutor does not sort tasks"));
                                }
                                task.run();
                                drop(_interval);
                            }
                            None => break,
                        }
                    }
                },
                recv(self.receiver) -> task => {
                    match task {
                        Ok(task) => {
                            let mut _interval = None;
                            if task.imp.priority() < some_executor::Priority::UserInteractive {
                                _interval = Some(logwise::perfwarn_begin_if!(logwise::Duration::from_millis(5), "StaticExecutor does not sort tasks"));
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
                    logwise::Level::DebugInternal,
                    logwise::log_enabled!(logwise::Level::DebugInternal),
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

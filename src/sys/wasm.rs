use crate::DrainNotify;
use crate::sys::wasm::static_executor::StaticExecutor;
use channel::{Receiver, Sender};
use some_executor::SomeStaticExecutor;
use some_executor::task::{DynSpawnedTask, TaskID};
use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, RawWaker, RawWakerVTable, Waker};
use wasm_bindgen::JsCast;

pub mod channel;
mod static_executor;

#[derive(Debug)]
struct Internal {
    drain_notify: Arc<DrainNotify>,
    thread_sender: Sender<TaskMessage>,
    thread_receiver: Receiver<TaskMessage>,
    pending_tasks: Arc<Mutex<HashMap<TaskID, crate::SpawnedTask>>>,
    size: Mutex<usize>,
}

#[derive(Debug, Clone)]
pub struct Executor {
    internal: Arc<Internal>,
}

impl PartialEq for Executor {
    fn eq(&self, _other: &Self) -> bool {
        Arc::ptr_eq(&self.internal, &_other.internal)
    }
}

impl Eq for Executor {}

impl std::hash::Hash for Executor {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.internal).hash(state);
    }
}

const WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
    |data| {
        let data = unsafe { Arc::from_raw(data as *const WakeInfo) };
        let data2 = data.clone();
        std::mem::forget(data);
        RawWaker::new(Arc::into_raw(data2) as *const (), &WAKER_VTABLE)
    },
    |data| {
        let data = unsafe { Arc::from_raw(data as *const WakeInfo) };
        data.wake();
        drop(data);
    },
    |data| {
        let data = unsafe { Arc::from_raw(data as *const WakeInfo) };
        data.wake();
        std::mem::forget(data);
    },
    |data| {
        let data = unsafe { Arc::from_raw(data as *const WakeInfo) };
        std::mem::drop(data);
    },
);

struct WakeInfo {
    thread_sender: Sender<TaskMessage>,
    inline_wake: AtomicBool,
    task_id: TaskID,
}

impl WakeInfo {
    fn wake(&self) {
        self.inline_wake.store(true, Ordering::Release);
        //send a message to the thread to resume the task
        self.thread_sender
            .send(TaskMessage::ResumeTask(self.task_id))
    }
}

#[derive(Debug)]
enum TaskMessage {
    NewTask(crate::SpawnedTask),
    ResumeTask(some_executor::task::TaskID),
    Shutdown,
}

struct Thread {
    static_executor: StaticExecutor,
    thread_receiver: channel::Receiver<TaskMessage>,
    thread_sender: channel::Sender<TaskMessage>,
    drain_notify: Arc<DrainNotify>,
    pending_tasks: Weak<Mutex<HashMap<some_executor::task::TaskID, crate::SpawnedTask>>>,
}
impl Thread {
    fn run(self) {
        //we need an async context here.
        //since wasm_thread doesn't support async entrypoints, let's use our static executor
        //to break out of the box
        //we must clone this out of self so we can move self into the future
        let executor = self.static_executor.clone();
        let boxed_executor: Box<dyn SomeStaticExecutor<ExecutorNotifier = Infallible>> =
            Box::new(executor.clone());
        some_executor::thread_executor::set_thread_static_executor_adapting_notifier(
            boxed_executor,
        );
        executor.spawn(self.run_async());
        //this will normally exit the thread, but that's handled by our static executor.
    }
    async fn run_async(mut self) {
        'all_tasks: loop {
            //wait for a task
            let message = self.thread_receiver.recv().await;
            //logwise::debuginternal_sync!("Thread::run_async received message: {message}",message=logwise::privacy::LogIt(&message));
            if let Some(message) = message {
                match message {
                    TaskMessage::NewTask(task) => {
                        self.poll_task(task);
                    }
                    TaskMessage::ResumeTask(task_id) => {
                        let pending_tasks = self
                            .pending_tasks
                            .upgrade()
                            .expect("Task resuming while shutting down?");
                        let task = pending_tasks.lock().unwrap().remove(&task_id);
                        if let Some(task) = task {
                            //we have a task to poll
                            self.poll_task(task);
                        } else {
                            logwise::debuginternal_sync!(
                                "Thread::run_async received ResumeTask for duplicate/nonpending task: {task}",
                                task = logwise::privacy::LogIt(&task_id)
                            );
                        }
                    }
                    TaskMessage::Shutdown => {
                        logwise::debuginternal_sync!(
                            "Thread::run_async received Shutdown message, exiting thread"
                        );
                        //we can exit the thread
                        break 'all_tasks;
                    }
                }
            }
        }
    }
    fn poll_task(&mut self, mut task: crate::SpawnedTask) {
        // logwise::debuginternal_sync!("Thread::poll_task: {task_id} {task_label}", task_id = logwise::privacy::LogIt(task.imp.task.task_id()), task_label = task.imp.task.label());
        //ensure task has an id

        //we could use spawn_local here, but it should be more efficient to use a work-stealing approach
        let wake_info = WakeInfo {
            thread_sender: self.thread_sender.clone(),
            inline_wake: AtomicBool::new(false),
            task_id: task.imp.task.task_id(),
        };
        let wake_info = Arc::new(wake_info);
        let move_wake_info = wake_info.clone();
        let raw_wake_info = Arc::into_raw(move_wake_info);
        let waker =
            unsafe { Waker::from_raw(RawWaker::new(raw_wake_info as *const (), &WAKER_VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let r = DynSpawnedTask::poll(task.imp.task.as_mut(), &mut cx, None);
        match r {
            std::task::Poll::Ready(_r) => {
                logwise::debuginternal_sync!(
                    "Task {task} finished",
                    task = logwise::privacy::LogIt(task.imp.task.task_id())
                );
                // task is done, we can notify the drain notify
                let old = self
                    .drain_notify
                    .running_tasks
                    .fetch_sub(1, std::sync::atomic::Ordering::Release);
                if old == 1 {
                    //if we were the last task, we can notify the drain notify
                    self.drain_notify.waker.wake();
                }
            }
            std::task::Poll::Pending => {
                // logwise::debuginternal_sync!("Task {task} yielded", task=logwise::privacy::LogIt(task.imp.task.task_id()));
                //note that we can get woken here, prior to inserting the task into the pending tasks
                let pending_tasks = self
                    .pending_tasks
                    .upgrade()
                    .expect("Task pending while shutting down?");
                let move_id = task.imp.task.task_id();
                pending_tasks.lock().unwrap().insert(move_id, task);
                //after this, check the inline wake to see if we missed that poll
                if wake_info.inline_wake.load(Ordering::Acquire) {
                    //if the inline wake was set, ensure a poll fires after the pending task insert!
                    self.thread_sender.send(TaskMessage::ResumeTask(move_id));
                }
            }
        }
    }
}

impl Executor {
    pub fn new(_name: String, threads: usize) -> Self {
        let drain_notify = Arc::new(DrainNotify::new());
        let pending_tasks = Arc::new(Mutex::new(HashMap::new()));

        let (thread_sender, thread_receiver) = channel::channel();

        for t in 0..threads {
            let thread_receiver = thread_receiver.clone();
            let drain_notify = drain_notify.clone();
            let thread_sender = thread_sender.clone();
            let pending_tasks = pending_tasks.clone();
            Self::spawn_thread(
                t,
                thread_receiver,
                drain_notify,
                &pending_tasks,
                thread_sender,
            );
        }

        Executor {
            internal: Arc::new(Internal {
                thread_sender,
                drain_notify,
                pending_tasks,
                size: Mutex::new(threads),
                thread_receiver,
            }),
        }
    }

    pub fn drain_notify(&self) -> &Arc<DrainNotify> {
        &self.internal.drain_notify
    }

    pub fn spawn_internal(&self, spawned_task: crate::SpawnedTask) {
        self.drain_notify()
            .running_tasks
            .fetch_add(1, Ordering::Relaxed);
        self.internal
            .thread_sender
            .send(TaskMessage::NewTask(spawned_task))
    }

    pub fn resize(&mut self, threads: usize) {
        let mut old_size = self.internal.size.lock().unwrap();
        if threads < *old_size {
            let shutdown_threads = *old_size - threads;
            for _ in 0..shutdown_threads {
                self.internal.thread_sender.send(TaskMessage::Shutdown);
            }
        } else if threads > *old_size {
            //we need to spawn new threads
            for t in *old_size..threads {
                Self::spawn_thread(
                    t,
                    self.internal.thread_receiver.clone(),
                    self.internal.drain_notify.clone(),
                    &self.internal.pending_tasks,
                    self.internal.thread_sender.clone(),
                );
            }
        }
        *old_size = threads;
    }
    fn spawn_thread(
        t: usize,
        thread_receiver: channel::Receiver<TaskMessage>,
        drain_notify: Arc<DrainNotify>,
        pending_tasks: &Arc<Mutex<HashMap<some_executor::task::TaskID, crate::SpawnedTask>>>,
        thread_sender: channel::Sender<TaskMessage>,
    ) {
        let pending_tasks = Arc::downgrade(pending_tasks);
        wasm_thread::Builder::new()
            .name("some_global_executor_{}".to_string() + &t.to_string())
            .spawn(move || {
                let t = Thread {
                    static_executor: StaticExecutor::new(),
                    drain_notify,
                    thread_receiver,
                    pending_tasks,
                    thread_sender,
                };
                t.run();
            })
            .unwrap();
    }
}

#[derive(Debug)]
pub struct SpawnedTask {
    task: Pin<Box<dyn DynSpawnedTask<Infallible>>>,
}

impl SpawnedTask {
    pub fn new(task: Box<dyn DynSpawnedTask<Infallible>>) -> Self {
        Self {
            task: Box::into_pin(task),
        }
    }
}

pub fn default_threadpool_size() -> usize {
    if let Some(window) = web_sys::window() {
        window.navigator().hardware_concurrency() as usize
    } else {
        let global = js_sys::global();
        if let Some(worker) = global.dyn_into::<web_sys::WorkerGlobalScope>().ok() {
            worker.navigator().hardware_concurrency() as usize
        } else {
            todo!("Not implemented");
        }
    }
}

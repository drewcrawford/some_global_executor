use std::cell::RefCell;
use std::collections::HashMap;
use crate::sys::wasm::static_executor::StaticExecutor;
use std::convert::Infallible;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Wake, Waker};
use some_executor::task::DynSpawnedTask;
use crate::{DrainNotify};
use channel::Sender;

pub mod channel;
mod static_executor;

#[derive(Debug)]
struct Internal {
    drain_notify: Arc<DrainNotify>,
    thread_sender: Sender<ThreadMessage>,
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
        let data = unsafe { Arc::from_raw(data as *const WakeInfo ) };
        let data2 = data.clone();
        std::mem::forget(data);
        RawWaker::new(Arc::into_raw(data2) as *const (), &WAKER_VTABLE)
    },
    |data| {
        let data = unsafe { Arc::from_raw(data as *const WakeInfo ) };
        data.wake();
        drop(data);
    },
    |data| {
        let data = unsafe { Arc::from_raw(data as *const WakeInfo ) };
        data.wake();
        std::mem::forget(data);
    },
    |data|  {
        let data = unsafe { Arc::from_raw(data as *const WakeInfo ) };
        std::mem::drop(data);
    },
);

struct WakeInfo {
    task_pending_id: usize,
    thread_sender: Sender<ThreadMessage>,
}

impl WakeInfo {
    fn wake(&self) {
        self.thread_sender.send(ThreadMessage::ResumeTask(self.task_pending_id))
    }
}

#[derive(Debug)]
enum ThreadMessage {
    NewTask(crate::SpawnedTask),
    ResumeTask(usize),
}



struct Thread {
    static_executor: StaticExecutor,
    thread_receiver: channel::Receiver<ThreadMessage>,
    thread_sender: channel::Sender<ThreadMessage>,
    drain_notify: Arc<DrainNotify>,
    next_pending_id: usize,
    pending_tasks: HashMap<usize, crate::SpawnedTask>,
}
impl Thread {
    fn run(self) {
        //we need an async context here.
        //since wasm_thread doesn't support async entrypoints, let's use our static executor
        //to break out of the box
        //we must clone this out of self so we can move self into the future
        let executor = self.static_executor.clone();
        executor.spawn(self.run_async());
        //this will normally exit the thread, but that's handled by our static executor.
    }
    async fn run_async(mut self) {
        'all_tasks: loop {
            //wait for a task
            let message = self.thread_receiver.recv().await;
            if let Some(mut message) = message {
                match message {
                    ThreadMessage::NewTask(mut task) => {
                        self.poll_task(task);
                    }
                    ThreadMessage::ResumeTask(id) => {
                        let task = self.pending_tasks.remove(&id);
                        if let Some(task) = task {
                            //we have a task to resume
                            self.poll_task(task);
                        } else {
                            //if we don't have a task, we can ignore this message
                            continue 'all_tasks;
                        }
                    }
                }

            }
        }
    }
    fn poll_task(&mut self, mut task: crate::SpawnedTask) {
        let task_pending_id = match task.imp.pending_id {
            Some(id) => id,
            None => {
                //if the task doesn't have a pending id, we need to assign one
                let next_id = self.next_pending_id;
                task.imp.pending_id = Some(next_id);
                self.next_pending_id += 1;
                next_id
            }
        };
        //we could use spawn_local here, but it should be more efficient to use a work-stealing approach
        let wake_info = WakeInfo {
            task_pending_id,
            thread_sender: self.thread_sender.clone(),
        };
        let wake_info = Arc::new(wake_info);
        let move_wake_info = wake_info.clone();
        let raw_wake_info = Arc::into_raw(move_wake_info);
        let waker = unsafe { Waker::from_raw(RawWaker::new(raw_wake_info as *const (), &WAKER_VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let r = DynSpawnedTask::poll(task.imp.task.as_mut(), &mut cx, None);
        match r {
            std::task::Poll::Ready(r) => {
                // task is done, we can notify the drain notify
                let old = self.drain_notify.running_tasks.fetch_sub(1, std::sync::atomic::Ordering::Release);
                if old == 1 {
                    //if we were the last task, we can notify the drain notify
                    self.drain_notify.waker.wake();
                }
            }
            std::task::Poll::Pending => {
                //ensure task has an id
                //task is pending, we need to store it
                self.pending_tasks.insert(task.imp.pending_id.unwrap(), task);
            }
        }
    }
}

impl Executor {
    pub fn new(_name: String, threads: usize) -> Self {
        let drain_notify = Arc::new(DrainNotify::new());

        let (thread_sender, thread_receiver) = channel::channel();

        for t in 0..threads {
            let thread_receiver = thread_receiver.clone();
            let drain_notify = drain_notify.clone();
            let thread_sender = thread_sender.clone();
            wasm_thread::Builder::new()
                .name("some_global_executor_{}".to_string() + &t.to_string())
                .spawn(move || {
                let t = Thread {
                    static_executor: StaticExecutor::new(),
                    drain_notify,
                    thread_receiver,
                    next_pending_id: 0,
                    pending_tasks: HashMap::new(),
                    thread_sender,
                };
                t.run();
            }).unwrap();
        }

        Executor {
            internal: Arc::new(Internal {
                thread_sender,
                drain_notify
            }),

        }
    }

    pub fn drain_notify(&self) -> &Arc<DrainNotify> {
        &self.internal.drain_notify
    }

    pub fn spawn_internal(&self, spawned_task: crate::SpawnedTask) {
        self.drain_notify().running_tasks.fetch_add(1, Ordering::Relaxed);
        self.internal.thread_sender.send(ThreadMessage::NewTask(spawned_task))
    }
}

#[derive(Debug)]
pub struct SpawnedTask {
    task: Pin<Box<dyn DynSpawnedTask<Infallible>>>,
    pending_id: Option<usize>,
}

impl SpawnedTask {
    pub fn new(task: Box<dyn DynSpawnedTask<Infallible>>) -> Self {
        Self { task: Box::into_pin(task), pending_id: None }
    }
}
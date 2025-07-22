use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc};
use std::task::Waker;
use crossbeam_channel::{select_biased, Receiver, Sender};
use some_executor::task::DynSpawnedTask;
use crate::waker::WakeInternal;
use crate::threadpool::ThreadMessage;

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

    pub fn run(mut self, task_sender: Sender<crate::SpawnedTask>, drain_notify: &crate::DrainNotify) {
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

    pub fn priority(&self) -> some_executor::Priority {
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
                            task.run(self.queue_task.clone(), &self.drain_notify);
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
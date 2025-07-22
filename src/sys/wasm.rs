use std::convert::Infallible;
use crossbeam_channel::{Receiver, Sender};
use some_executor::task::DynSpawnedTask;
use crate::threadpool::ThreadMessage;

#[derive(Debug)]
pub struct SpawnedTask {}

impl SpawnedTask {
    pub fn new(_task: Box<dyn DynSpawnedTask<Infallible>>) -> Self {
        todo!("WASM SpawnedTask::new not implemented")
    }

    pub fn run(self, _task_sender: Sender<crate::SpawnedTask>, _drain_notify: &crate::DrainNotify) {
        todo!("WASM SpawnedTask::run not implemented")
    }

    pub fn task_id(&self) -> impl std::fmt::Debug {
        todo!("WASM SpawnedTask::task_id not implemented")
    }

    pub fn priority(&self) -> some_executor::Priority {
        todo!("WASM SpawnedTask::priority not implemented")
    }
}

#[derive(Debug)]
pub struct Thread {}

impl Thread {
    pub fn new(
        _receiver: crossbeam_channel::Receiver<crate::SpawnedTask>,
        _queue_task: Sender<crate::SpawnedTask>,
        _drain_notify: std::sync::Arc<crate::DrainNotify>,
    ) -> Self {
        todo!("WASM Thread::new not implemented")
    }

    pub fn run(self, _receiver: Receiver<ThreadMessage>) {
        todo!("WASM Thread::run not implemented")
    }
}
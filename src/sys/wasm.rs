use std::convert::Infallible;
use std::sync::Arc;
use some_executor::task::DynSpawnedTask;
use crate::DrainNotify;

pub mod channel;

#[derive(Debug, Clone)]
pub struct Executor {
    drain_notify: Arc<DrainNotify>,
}

impl PartialEq for Executor {
    fn eq(&self, _other: &Self) -> bool {
        todo!()
    }
}

impl Eq for Executor {}

impl std::hash::Hash for Executor {
    fn hash<H: std::hash::Hasher>(&self, _state: &mut H) {
        todo!()
    }
}

impl Executor {
    pub fn new(_name: String, _threads: usize) -> Self {
        let drain_notify = Arc::new(DrainNotify::new());
        Self { drain_notify }
    }

    pub fn drain_notify(&self) -> &Arc<DrainNotify> {
        &self.drain_notify
    }

    pub fn spawn_internal(&self, _spawned_task: crate::SpawnedTask) {
        todo!()
    }
}

#[derive(Debug)]
pub struct SpawnedTask {
    _task: Box<dyn DynSpawnedTask<Infallible>>,
}

impl SpawnedTask {
    pub fn new(task: Box<dyn DynSpawnedTask<Infallible>>) -> Self {
        Self { _task: task }
    }
}
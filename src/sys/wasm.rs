mod channel;

use std::convert::Infallible;
use std::sync::Arc;
use some_executor::task::DynSpawnedTask;
use crate::threadpool::{ThreadBuilder, ThreadFn, ThreadMessage};
use channel::{Sender, Receiver};

#[derive(Debug)]
pub struct SpawnedTask {
    imp: Box<dyn DynSpawnedTask<Infallible>>,
}

impl SpawnedTask {
    pub fn new(task: Box<dyn DynSpawnedTask<Infallible>>) -> Self {
        Self {
            imp: task,
        }
    }

    pub(crate) fn run(self, _task_sender: Sender<crate::SpawnedTask>, _drain_notify: &crate::DrainNotify) {
        todo!("WASM SpawnedTask::run not implemented")
    }

    pub fn task_id(&self) -> impl std::fmt::Debug {
        self.imp.task_id()
    }

    pub(crate) fn priority(&self) -> some_executor::Priority {
        self.imp.priority()
    }
}

#[derive(Debug)]
pub struct Thread {
    receiver: crossbeam_channel::Receiver<crate::SpawnedTask>,
}

impl Thread {
    pub fn new(
        receiver: crossbeam_channel::Receiver<crate::SpawnedTask>,
        _queue_task: Sender<crate::SpawnedTask>,
        _drain_notify: std::sync::Arc<crate::DrainNotify>,
    ) -> Self {
        Self {
            receiver,
        }
    }

    pub fn run(self, _receiver: Receiver<ThreadMessage>) {
        //this static compile check is to ensure that we handle appropriate messages
        fn _static_compile_check(t: ThreadMessage) {
            match t {
                ThreadMessage::Shutdown => {
                    // we don't need to handle this one on wasm
                }
            }
        }

        todo!();

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
        todo!("WASM Builder::build not implemented")
    }
}

impl ThreadFn for ThreadImpl {
    fn run(self, receiver: Receiver<ThreadMessage>) {
        todo!("WASM ThreadImpl::run not implemented")
    }
}

#[derive(Debug)]
pub struct Threadpool<B> {
    _phantom: std::marker::PhantomData<B>,
}

impl<B> Threadpool<B> {
    pub fn new(name: String, size: usize, thread_builder: B) -> Self
    where B: crate::threadpool::ThreadBuilder, {
        //on wasm, we want to lean on the browser's executor if possible
        //for that reason, we want to  use a sync-async channel as our primitive
        let (sender,receiver) = channel::channel();
        let mut vec = Vec::with_capacity(size);
        for t in 0..size {
            let receiver = receiver.clone();
            let thread_fn = thread_builder.build();
            let handle = Self::build_thread(&name, t, receiver, thread_fn);
            vec.push(handle);
        }
        todo!();

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

    pub async fn resize(&mut self, _size: usize)
    where B: crate::threadpool::ThreadBuilder {
        todo!("WASM Threadpool::resize not implemented")
    }
}
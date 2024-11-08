use std::sync::Arc;
use crossbeam_channel::Receiver;
use crate::threadpool::{ThreadBuilder, ThreadFn, ThreadMessage, Threadpool};

mod threadpool;

struct Builder {

}
struct Thread {

}
impl ThreadBuilder for Builder {
    type ThreadFn = Thread;
    fn build(&mut self) -> Self::ThreadFn {
        Thread {

        }
    }
}
impl ThreadFn for Thread {
    fn run(self, receiver: Receiver<ThreadMessage>) {
        loop {
            match receiver.recv() {
                Ok(ThreadMessage::Shutdown) => {
                    break;
                }
                Err(_) => {
                    break;
                }
            }
        }
    }
}

pub struct Executor {
    threadpool: Threadpool<Builder>,
}


impl Executor {
    pub fn new(name: String, size: usize) -> Self {
        let builder = Builder {};
        let threadpool = Threadpool::new(name, size, builder);
        Self {
            threadpool
        }
    }

    pub fn new_default() -> Self {
        let builder = Builder {};
        let threadpool = Threadpool::new_default("default".to_string(), builder);
        Self {
            threadpool
        }
    }

    pub fn join(self) {
        self.threadpool.join();
    }

}

#[cfg(test)] mod tests {
    #[test] fn new() {
        let e = super::Executor::new("test".to_string(), 4);
        e.join();
    }
}
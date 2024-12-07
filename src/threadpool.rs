use std::sync::{Mutex};
use crossbeam_channel::{Receiver, Sender};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod sys {
    pub use std::thread::*;
}

#[cfg(target_arch = "wasm32")]
pub(crate) mod sys {
    pub use wasm_thread::*;
}

pub trait ThreadBuilder {
    type ThreadFn: ThreadFn;
    fn build(&mut self) -> Self::ThreadFn;
}

pub trait ThreadFn: Send + 'static {
    fn run(self, receiver: Receiver<ThreadMessage>);
}

impl<T> ThreadFn for T
where T: Fn(Receiver<ThreadMessage>) -> () + Send + 'static {
    fn run(self, receiver: Receiver<ThreadMessage>) {
        self(receiver);
    }
}

impl<B,T> ThreadBuilder for B
where B: Fn() -> T,
T: ThreadFn {
    type ThreadFn = T;

    fn build(&mut self) -> Self::ThreadFn {
        self()
    }
}

#[derive(Debug)]
pub struct Threadpool<B> {
    vec: Mutex<Vec<sys::JoinHandle<()>>>,
    requested_threads: usize,
    sender: Sender<ThreadMessage>,
    receiver: Receiver<ThreadMessage>,
    name: String,
    thread_builder: B,
}

pub enum ThreadMessage {
    Shutdown,
}

impl<B> Threadpool<B> {
    pub fn new(name: String, size: usize, mut thread_builder: B) -> Self
    where B: ThreadBuilder, {
        let mut vec = Vec::with_capacity(size);
        let (sender,receiver) = crossbeam_channel::unbounded();
        for t in 0..size {
            let receiver = receiver.clone();
            let thread_fn = thread_builder.build();
            let handle = Self::build_thread(&name, t, receiver, thread_fn);
            vec.push(handle);
        }
        let vec = Mutex::new(vec);
        Threadpool { vec, sender, receiver,name,thread_builder, requested_threads: size }
    }

    fn build_thread<T: ThreadFn>(name: &str, thread_no: usize, receiver: Receiver<ThreadMessage>,thread_fn: T) -> sys::JoinHandle<()> {
        let name = format!("some_global_executor {}-{}", name, thread_no);
        sys::Builder::new()
            .name(name)
            .spawn(move || {
                let c = logwise::context::Context::new_task(None, "some_global_executor threadpool");
                c.set_current();
                thread_fn.run(receiver);
            })
            .unwrap()
    }




    pub async fn resize(&mut self, size: usize)
    where B: ThreadBuilder {
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
                self.sender.send(ThreadMessage::Shutdown).unwrap();
            }
            //gc
            lock.retain(|handle| !handle.is_finished());

        }
    }


}

#[cfg(test)] mod tests {
    use crate::threadpool::{Threadpool};


    #[test_executors::async_test]
    async fn resize() {
        logwise::context::Context::reset("resize");
        let builder = || {
            |_| {
                println!("hi");
            }
        };

        let mut threadpool = Threadpool::new("resize".to_string(), 4, builder);
        threadpool.resize(2).await;
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn test_num_cpus() {
        logwise::info_sync!("num_cpus: {cpus} physical: {physical}", cpus=num_cpus::get(),physical=num_cpus::get_physical());
    }
}
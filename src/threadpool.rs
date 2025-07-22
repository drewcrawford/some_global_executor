use crossbeam_channel::Receiver;

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
    imp: crate::sys::Threadpool<B>,
}

pub enum ThreadMessage {
    Shutdown,
}

impl<B> Threadpool<B> {
    pub fn new(name: String, size: usize, thread_builder: B) -> Self
    where B: ThreadBuilder, {
        Self {
            imp: crate::sys::Threadpool::new(name, size, thread_builder),
        }
    }

    pub async fn resize(&mut self, size: usize)
    where B: ThreadBuilder {
        self.imp.resize(size).await;
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
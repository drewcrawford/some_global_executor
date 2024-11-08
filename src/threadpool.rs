use std::sync::Arc;
use std::thread::JoinHandle;
use crossbeam_channel::{Receiver, Sender};

trait ThreadBuilder {
    type ThreadFn: ThreadFn;
    fn build(&mut self) -> Self::ThreadFn;
}

trait ThreadFn: Send + 'static {
    fn run(self);
}

impl<T> ThreadFn for T
where T: Fn() -> () + Send + 'static {
    fn run(self) {
        self();
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

pub struct Threadpool<B> {
    vec: Vec<JoinHandle<()>>,
    sender: Sender<ThreadMessage>,
    receiver: Receiver<ThreadMessage>,
    name: String,
    thread_builder: B,
}

enum ThreadMessage {
    Shutdown,
}

impl<B> Threadpool<B> {
    /**
    Creates a threadpool sized to the number of CPUs.
*/
    pub fn new_default(name: String, thread_builder: B) -> Self
    where B: ThreadBuilder, {
        Self::new(name, num_cpus::get(), thread_builder)
    }
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
        Threadpool { vec, sender, receiver,name,thread_builder }
    }

    fn build_thread<T: ThreadFn>(name: &str, thread_no: usize, receiver: Receiver<ThreadMessage>,thread_fn: T) -> JoinHandle<()> {
        let name = format!("some_global_executor {}-{}", name, thread_no);
        std::thread::Builder::new()
            .name(name)
            .spawn(move || {
                let c = logwise::context::Context::new_task(None, "some_global_executor threadpool");
                c.set_current();
                thread_fn.run();
            })
            .unwrap()
    }


    pub fn join(self) {
        for _ in &self.vec {
            self.sender.send(ThreadMessage::Shutdown).unwrap();
        }
        for handle in self.vec {
            handle.join().unwrap();
        }
    }

    pub fn resize(&mut self, size: usize)
    where B: ThreadBuilder {
        let old_size = self.vec.len();
        if size > old_size {
            for t in old_size..size {
                let receiver = self.receiver.clone();
                let handle = Self::build_thread(&self.name, t, receiver,self.thread_builder.build());
                self.vec.push(handle);
            }
        } else {
            for _ in size..old_size {
                self.sender.send(ThreadMessage::Shutdown).unwrap();
            }
            let mut _interval = None;
            while self.vec.len() > size {
                self.vec.retain(|handle| !handle.is_finished());
                if _interval.is_none() {
                    _interval = Some(logwise::perfwarn_begin!("Threadpool::resize busyloop"));
                }
                std::hint::spin_loop();
            }
            _interval = None;
        }
    }


}

#[cfg(test)] mod tests {
    use crate::threadpool::{ThreadBuilder, Threadpool};


    #[test] fn resize() {
        logwise::context::Context::reset("resize");
        let builder = || {
            || {
                println!("hi");
            }
        };

        let mut threadpool = Threadpool::new("resize".to_string(), 4, builder);
        threadpool.resize(2);
        threadpool.join();
    }

    #[test] fn test_num_cpus() {
        println!("num_cpus: {}", num_cpus::get());
        println!("num_cpus_physical: {}", num_cpus::get_physical());
    }
}
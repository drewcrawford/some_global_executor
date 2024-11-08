use std::thread::JoinHandle;
use crossbeam_channel::{Receiver, Sender};

pub struct Threadpool {
    vec: Vec<JoinHandle<()>>,
    sender: Sender<ThreadMessage>,
    receiver: Receiver<ThreadMessage>,
    name: String,
}

enum ThreadMessage {
    Run(Box<dyn FnOnce() + Send>),
    End,
}

impl Threadpool {
    pub fn new(name: String, size: usize) -> Threadpool {
        let mut vec = Vec::with_capacity(size);
        let (sender,receiver) = crossbeam_channel::unbounded();
        for t in 0..size {
            let receiver = receiver.clone();
            let handle = Self::build_thread(&name, t, receiver);
            vec.push(handle);
        }
        Threadpool { vec, sender, receiver,name }
    }

    fn build_thread(name: &str, thread_no: usize, receiver: Receiver<ThreadMessage>) -> JoinHandle<()> {
        let name = format!("some_global_executor {}-{}", name, thread_no);
        std::thread::Builder::new()
            .name(name)
            .spawn(move || {
                let c = logwise::context::Context::new_task(None, "some_global_executor threadpool");
                c.set_current();
                loop {
                    match receiver.recv() {
                        Ok(ThreadMessage::Run(f)) => f(),
                        Ok(ThreadMessage::End) => break,
                        Err(_) => break,
                    }
                }
            })
            .unwrap()
    }

    pub fn submit<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.sender.send(ThreadMessage::Run(Box::new(f))).unwrap();
    }

    pub fn join(self) {
        for handle in &self.vec {
            self.sender.send(ThreadMessage::End).unwrap();
        }
        for handle in self.vec {
            handle.join().unwrap();
        }
    }

    pub fn resize(&mut self, size: usize) {
        let old_size = self.vec.len();
        if size > old_size {
            for t in old_size..size {
                let receiver = self.receiver.clone();
                let handle = Self::build_thread(&self.name, t, receiver);
                self.vec.push(handle);
            }
        } else {
            for _ in size..old_size {
                self.sender.send(ThreadMessage::End).unwrap();
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
    use crate::threadpool::Threadpool;

    #[test] fn new() {
        logwise::context::Context::reset("new");
        let t = Threadpool::new("test".to_string(), 4);
        t.submit(|| {
            println!("Hello from thread 1");
        });
        t.submit(|| {
            println!("Hello from thread 2");
        });
        t.join();
    }

    #[test] fn resize() {
        logwise::context::Context::reset("resize");
        let mut t = Threadpool::new("test".to_string(), 4);
        t.submit(|| {
            println!("Hello from thread 1");
        });
        t.submit(|| {
            println!("Hello from thread 2");
        });
        t.resize(2);
        t.submit(|| {
            println!("Hello from thread 3");
        });
        t.submit(|| {
            println!("Hello from thread 4");
        });
        t.resize(1);
        t.join();
    }
}
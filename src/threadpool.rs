use std::thread::JoinHandle;
use crossbeam_channel::Sender;

pub struct Threadpool {
    vec: Vec<JoinHandle<()>>,
    sender: Sender<ThreadMessage>,
}

enum ThreadMessage {
    Run(Box<dyn FnOnce() + Send>),
    End,
}

impl Threadpool {
    pub fn new(name: &str, size: usize) -> Threadpool {
        let mut vec = Vec::with_capacity(size);
        let (sender,receiver) = crossbeam_channel::unbounded();
        for t in 0..size {
            let receiver = receiver.clone();
            let handle = std::thread::Builder::new()
                .name(format!("some_global_executor {}-{}", name, t))
                .spawn(move || {
                    loop {
                        match receiver.recv() {
                            Ok(ThreadMessage::Run(f)) => f(),
                            Ok(ThreadMessage::End) => break,
                            Err(_) => break,
                        }
                    }
                })
                .unwrap();
            vec.push(handle);
        }
        Threadpool { vec, sender }
    }

    pub fn join(self) {
        for handle in &self.vec {
            self.sender.send(ThreadMessage::End).unwrap();
        }
        for handle in self.vec {
            handle.join().unwrap();
        }
    }


}

#[cfg(test)] mod tests {
    use crate::threadpool::Threadpool;

    #[test] fn new() {
        let t = Threadpool::new("test", 4);
        t.join();
    }
}
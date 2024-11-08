use std::thread::JoinHandle;

pub struct Threadpool {
    vec: Vec<JoinHandle<()>>,
}

impl Threadpool {
    pub fn new(name: &str, size: usize) -> Threadpool {
        let mut vec = Vec::with_capacity(size);
        for t in 0..size {
            let handle = std::thread::Builder::new()
                .name(format!("some_global_executor {}-{}", name, t))
                .spawn(|| {
                    println!("Hello from a thread!");
                })
                .unwrap();
            vec.push(handle);
        }
        Threadpool { vec }
    }

    pub fn join(self) {
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
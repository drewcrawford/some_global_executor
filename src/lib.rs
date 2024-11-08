use std::sync::Arc;
use crate::threadpool::Threadpool;

mod threadpool;

struct Builder {

}

pub struct Executor {
    threadpool: Threadpool<Builder>,
}


impl Executor {
    fn make_thread_fn() -> impl Fn() -> () + Send {
        let thread_fn = || {
            todo!();
        };
        thread_fn
    }
    pub fn new(name: String, size: usize) -> Self {
        todo!()
    }

    pub fn new_default() -> Self {
        todo!()
    }


}

#[cfg(test)] mod tests {
    #[test] fn new() {
        let _ = super::Executor::new("test".to_string(), 4);
    }
}
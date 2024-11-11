use std::sync::{Mutex};
use crossbeam_channel::{Receiver, Sender};

#[cfg(not(target_arch = "wasm32"))]
mod sys {
    pub use std::thread::*;
}

#[cfg(target_arch = "wasm32")]
mod sys {
    use std::sync::Arc;
    use std::marker::PhantomData;
    use std::sync::atomic::AtomicBool;
    use wasm_bindgen::prelude::wasm_bindgen;

    struct WebWorkerContext {
        func: Box<dyn FnOnce() + Send>,
    }

    pub struct Builder {
        name: Option<String>
    }
    #[wasm_bindgen]
    pub fn wasm_thread_entry_point(ptr: u32) {
        let ctx = unsafe { Box::from_raw(ptr as *mut WebWorkerContext) };
        (ctx.func)();
    }
    fn get_wasm_bindgen_shim_script_path() -> String {
        js_sys::eval(include_str!("js/script_path.js"))
            .unwrap()
            .as_string()
            .unwrap()
    }
    impl Builder {
        pub fn new() -> Self {
            Builder {
                name: None
            }
        }
        pub fn name(mut self, name: String) -> Self {
            self.name = Some(name);
            self
        }
        pub fn spawn<F>(self, f: F) -> std::thread::Result<JoinHandle<()>> where F: FnOnce() + Send + 'static {
            use wasm_bindgen::JsValue;
            let finished = Arc::new(AtomicBool::new(false));
            let move_finished = finished.clone();
            let closure = move || {
                f();
                move_finished.store(true, std::sync::atomic::Ordering::Relaxed);
                logwise::debuginternal_sync!("set finished to TRUE");
            };

            let script = include_str!("js/worker.js")
                .replace("WASM_BINDGEN_SHIM_URL", &get_wasm_bindgen_shim_script_path());

            let arr = js_sys::Array::new();
            arr.set(0, JsValue::from_str(&script));
            let blob = web_sys::Blob::new_with_str_sequence(&arr).unwrap();
            let url = web_sys::Url::create_object_url_with_blob(
                &blob
                    .slice_with_f64_and_f64_and_content_type(0.0, blob.size(), "text/javascript")
                    .unwrap(),
            )
                .unwrap();
            let options = web_sys::WorkerOptions::new();
            options.set_type(web_sys::WorkerType::Module);
            let worker = web_sys::Worker::new_with_options(&url, &options).unwrap();
            let ptr = Box::into_raw(Box::new(
                WebWorkerContext {
                    func: Box::new(closure),
                }
            ));
            let msg = js_sys::Array::new();
            msg.push(&wasm_bindgen::module());
            msg.push(&wasm_bindgen::memory());
            msg.push(&JsValue::from(ptr as u32));
            worker.post_message(&msg).unwrap();
            std::mem::forget(worker); //TODO!!!
            worker.g
            Ok(JoinHandle {
                phantom_data: PhantomData,
                finished
            })
        }
    }
    #[derive(Debug)]
    pub struct JoinHandle<R> {
        phantom_data: PhantomData<R>,
        finished: Arc<AtomicBool>
    }
    impl<R> JoinHandle<R> {
        pub fn join(self) -> std::thread::Result<R> {
            todo!()
        }
        pub fn is_finished(&self) -> bool {
            self.finished.load(std::sync::atomic::Ordering::Relaxed)
        }
    }
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
        Threadpool { vec, sender, receiver,name,thread_builder }
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


    pub fn join(&self) {
        let mut lock = self.vec.lock().unwrap();
        for _ in lock.iter() {
            self.sender.send(ThreadMessage::Shutdown).unwrap();
        }
        for handle in lock.drain(..) {
            handle.join().unwrap();
        }
    }

    pub fn resize(&mut self, size: usize)
    where B: ThreadBuilder {
        let mut lock = self.vec.lock().unwrap();
        let old_size = lock.len();
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
            let mut _interval = None;
            let mut debug = 0;
            while lock.len() > size {
                lock.retain(|handle| !handle.is_finished());
                if _interval.is_none() {
                    _interval = Some(logwise::perfwarn_begin!("Threadpool::resize busyloop"));
                }
                std::hint::spin_loop();
                std::thread::yield_now();
                debug += 1;
                if debug > 100000 {
                    panic!("Threadpool::resize busyloop failed to work");
                }
            }
            _interval = None;
        }
    }


}

#[cfg(test)] mod tests {
    use crate::threadpool::{Threadpool};


    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn resize() {
        logwise::context::Context::reset("resize");
        let builder = || {
            |_| {
                println!("hi");
            }
        };

        let mut threadpool = Threadpool::new("resize".to_string(), 4, builder);
        threadpool.resize(2);
        threadpool.join();
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn test_num_cpus() {
        println!("num_cpus: {}", num_cpus::get());
        println!("num_cpus_physical: {}", num_cpus::get_physical());
    }
}
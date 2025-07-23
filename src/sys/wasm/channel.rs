/*!
This is a custom channel type.  We currently use this only on wasm, but there's no reason it coudln't
be extracted to a more general crate.

It provides
* async recv
* sync write
* single producer
* multi consumer
* non-broadcast (each message is consumed by one consumer)
* infinite buffer
 */

use std::sync::Weak;
use std::sync::Mutex;
use std::sync::Arc;

struct SharedLocked<T> {
    buffer: Vec<T>,
    resume: Vec<r#continue::Sender<T>>,
}

struct Shared<T> {
    locked: Mutex<SharedLocked<T>>,
}

pub struct Sender<T> {
    shared: Arc<Shared<T>>,
}

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared {
        locked: Mutex::new(SharedLocked {
            buffer: Vec::new(),
            resume: Vec::new(),
        }),
    });
    let sender = Sender { shared: shared.clone() };
    let receiver = Receiver { shared: Arc::downgrade(&shared) };
    (sender, receiver)
}

impl <T> Sender<T> {
    pub fn send(&self, value: T)  {
        // Implementation for sending a value
        let mut lock = self.shared.locked.lock().unwrap();
        if let Some(resume) = lock.resume.pop() {
            drop(lock);
            resume.send(value);
        } else {
            lock.buffer.push(value);
        }
    }
}

#[derive(Clone)]
pub struct Receiver<T> {
    shared: Weak<Shared<T>>,
}

impl <T> Receiver<T> {
    pub async fn recv(&self) -> Option<T> {
        if let Some(shared) = self.shared.upgrade() {
            let mut lock = shared.locked.lock().unwrap();
            if let Some(value) = lock.buffer.pop() {
                Some(value)
            }
            else {
                let (send,fut) = r#continue::continuation();
                lock.resume.push(send);
                drop(lock);
                Some(fut.await)
            }
        } else {
            None // Shared data has been dropped
        }
    }
}
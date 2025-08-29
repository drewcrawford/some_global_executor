/*!
This is a custom channel type.  We currently use this only on wasm, but there's no reason it coudln't
be extracted to a more general crate.

It provides
* async recv
* sync write
* multi producer
* multi consumer
* non-broadcast (each message is consumed by one consumer)
* infinite buffer
 */

use std::sync::Arc;
use std::sync::Weak;
use wasm_safe_mutex::Mutex;

#[derive(Debug)]
struct SharedLocked<T> {
    buffer: Vec<T>,
    resume: Vec<r#continue::Sender<T>>,
}

#[derive(Debug)]
struct Shared<T> {
    locked: Mutex<SharedLocked<T>>,
}

#[derive(Debug)]
pub struct Sender<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Sender {
            shared: self.shared.clone(),
        }
    }
}

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared {
        locked: Mutex::new(SharedLocked {
            buffer: Vec::new(),
            resume: Vec::new(),
        }),
    });
    let sender = Sender {
        shared: shared.clone(),
    };
    let receiver = Receiver {
        shared: Arc::downgrade(&shared),
    };
    (sender, receiver)
}

impl<T> Sender<T> {
    pub fn send(&self, value: T) {
        let resume = self.shared.locked.with_mut_sync(|data| {
            if let Some(resume) = data.resume.pop() {
                Some((resume, value))
            } else {
                data.buffer.push(value);
                None
            }
        });
        match resume {
            Some((resume, value)) => {
                // If we had a continuation, send the value to it
                resume.send(value);
            }
            None => {
                // Otherwise, just store the value in the buffer
            }
        }
    }
}

#[derive(Debug)]
pub struct Receiver<T> {
    shared: Weak<Shared<T>>,
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Receiver {
            shared: self.shared.clone(),
        }
    }
}

impl<T> Receiver<T> {
    pub async fn recv(&self) -> Option<T> {
        if let Some(shared) = self.shared.upgrade() {
            let r = shared
                .locked
                .with_mut_async(|data| {
                    if let Some(value) = data.buffer.pop() {
                        Ok(value)
                    } else {
                        // If the buffer is empty, we need to wait for a sender
                        let (send, fut) = r#continue::continuation();
                        data.resume.push(send);
                        Err(fut)
                    }
                })
                .await;
            match r {
                Ok(value) => Some(value),
                Err(send) => {
                    // If we had a continuation, we need to wait for it
                    let fut = send.await;
                    Some(fut)
                }
            }
        } else {
            None // Shared data has been dropped
        }
    }
}

use std::mem::forget;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::task::{RawWaker, RawWakerVTable};

const NOT_WOKEN: u64 = u64::MAX;
const WOKEN: u64 = u64::MAX - 1;
const INVALID_BOX: u64 = WOKEN;

struct AlertFn {
    alert: Box<dyn FnOnce() + Send + 'static>
}

impl AlertFn {
    fn new<F>(alert: F) -> AlertFn where F: FnOnce() + 'static + Send {
        AlertFn {
            alert: Box::new(alert)
        }
    }
}

#[derive(Debug)]
pub struct WakeInternal {
    //this contains either NOT_WOKEN, WOKEN, or an Box pointer.
    inline_wake: AtomicU64
}

impl WakeInternal {
    fn new() -> WakeInternal {
        WakeInternal {
            inline_wake: AtomicU64::new(NOT_WOKEN)
        }
    }

    fn wake_by_ref(&self) {
        match self.inline_wake.compare_exchange(NOT_WOKEN, WOKEN, std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed) {
            Ok(_) => {},
            Err(NOT_WOKEN) => {/* spurious? */},
            Err(WOKEN) => {
                //multiple wakes, probably fine
            },
            Err(boxed_fn) => {
                match self.inline_wake.compare_exchange(boxed_fn, WOKEN, std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed) {
                    Ok(_) => {
                        let b = unsafe { Box::from_raw(boxed_fn as *mut AlertFn) };
                        (b.alert)();
                    },
                    Err(WOKEN) => {
                        //multiple wakes, probably fine
                    },
                    Err(NOT_WOKEN) => {
                        //probably try again?
                        self.wake_by_ref();
                    },
                    Err(_) => {
                        //try again?
                        self.wake_by_ref();
                    }
                }
            }
        }
    }

    pub fn reset(&self) {
        self.inline_wake.store(NOT_WOKEN,std::sync::atomic::Ordering::Relaxed);
    }

    pub fn check_wake<F>(&self,alert: F) where F: FnOnce() + 'static + Send {
        let boxed_alert = Box::new(AlertFn::new(alert));
        let raw_alert = Box::into_raw(boxed_alert) as u64;
        assert!(raw_alert < INVALID_BOX);
        match self.inline_wake.compare_exchange(NOT_WOKEN, raw_alert, std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed) {
            Ok(_) => {},
            Err(NOT_WOKEN) => {
                unreachable!("Spurious wakeup?")
            },
            Err(WOKEN) => {
                //run alert inline instead
                let b = unsafe { Box::from_raw(raw_alert as *mut AlertFn) };
                (b.alert)();
            },
            Err(_) => {
                let b = unsafe { Box::from_raw(raw_alert as *mut AlertFn) };
                drop(b);
            }
        }
    }
}

impl Drop for WakeInternal {
    fn drop(&mut self) {
        let inline_wake = self.inline_wake.load(std::sync::atomic::Ordering::Relaxed);
        if inline_wake == WOKEN || inline_wake == NOT_WOKEN {
            
        }
        else {
            let b = unsafe { Box::from_raw(inline_wake as *mut ()) };
            drop(b);
        }

    }
}

const VTABLE: RawWakerVTable = RawWakerVTable::new(
    |data| {
        let a = unsafe{Arc::from_raw(data as *const WakeInternal)};
        let b = a.clone();
        std::mem::forget(a);
        RawWaker::new(Arc::into_raw(b) as *const (), &VTABLE)


    },
    |data| {
        let a = unsafe{Arc::from_raw(data as *const WakeInternal)};
        a.wake_by_ref();
    },
    |data| {
        let a = unsafe{Arc::from_raw(data as *const WakeInternal)};
        a.wake_by_ref();
        forget(a);
    },
    |data| {
        drop(unsafe{Arc::from_raw(data as *const WakeInternal)});
    },
);

pub fn task_waker() -> (std::task::Waker,Arc<WakeInternal>) {
    unsafe {
        let wake_internal = Arc::new(WakeInternal::new());
        (std::task::Waker::from_raw(std::task::RawWaker::new(Arc::into_raw(wake_internal.clone()) as *const (), &VTABLE)),wake_internal)
    }
}
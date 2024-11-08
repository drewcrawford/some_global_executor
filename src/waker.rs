use std::mem::forget;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::task::{RawWaker, RawWakerVTable};

const NOT_WOKEN: u64 = 0;
const WOKEN_INLINE: u64 = 1;


struct WakeInternal {
    //this contains either NOT_WOKEN, WOKEN_INLINE, or an Box pointer.
    inline_wake: AtomicU64
}

impl WakeInternal {
    fn new() -> WakeInternal {
        WakeInternal {
            inline_wake: AtomicU64::new(NOT_WOKEN)
        }
    }

    fn wake_by_ref(&self) {
        todo!()
    }
}

impl Drop for WakeInternal {
    fn drop(&mut self) {
        let inline_wake = self.inline_wake.load(std::sync::atomic::Ordering::Relaxed);
        if inline_wake == WOKEN_INLINE {
            return;
        }
        if inline_wake == NOT_WOKEN {
            return;
        }
        let b = unsafe { Box::from_raw(inline_wake as *mut ()) };
        drop(b);
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

pub fn task_waker() -> std::task::Waker {
    unsafe {
        let wake_internal = Arc::new(WakeInternal::new());
        std::task::Waker::from_raw(std::task::RawWaker::new(Arc::into_raw(wake_internal) as *const (), &VTABLE))
    }
}
use std::sync::Arc;
use std::task::RawWakerVTable;

struct WakeInternal {

}

const VTABLE: RawWakerVTable = RawWakerVTable::new(
    |data| {
        todo!()
    },
    |data| {
        todo!()
    },
    |data| {
        todo!()
    },
    |data| {
        todo!()
    },
);

pub fn task_waker() -> std::task::Waker {
    unsafe {
        let wake_internal = Arc::new(WakeInternal {

        });
        std::task::Waker::from_raw(std::task::RawWaker::new(Arc::into_raw(wake_internal) as *const (), &VTABLE))
    }
}
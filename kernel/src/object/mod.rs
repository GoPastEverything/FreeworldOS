use alloc::sync::Arc;

pub mod counter;
pub mod handle;
pub mod module;
pub mod process;

use counter::CounterObject;
use handle::{Handle, HandleError, Rights};

pub enum FwObject {
    Counter(CounterObject),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectError {
    Handle(HandleError),
    WrongObjectType,
    SelfTestFailed,
}

impl From<HandleError> for ObjectError {
    fn from(value: HandleError) -> Self {
        Self::Handle(value)
    }
}

pub fn init() -> Result<(), ObjectError> {
    handle::init()?;
    crate::arch::serial::println("  object: FW handle table online slots=64");
    Ok(())
}

pub fn create_counter(initial: u64, rights: Rights) -> Result<Handle, ObjectError> {
    let object = Arc::new(FwObject::Counter(CounterObject::new(initial)));
    Ok(handle::insert(object, rights)?)
}

pub fn counter_read(handle_value: Handle) -> Result<u64, ObjectError> {
    let object = handle::get(handle_value, Rights::READ)?;

    match object.as_ref() {
        FwObject::Counter(counter) => Ok(counter.read()),
    }
}

pub fn counter_increment(handle_value: Handle) -> Result<u64, ObjectError> {
    let object = handle::get(handle_value, Rights::WRITE)?;

    match object.as_ref() {
        FwObject::Counter(counter) => Ok(counter.increment()),
    }
}

pub fn close(handle_value: Handle) -> Result<(), ObjectError> {
    Ok(handle::close(handle_value)?)
}

#[cfg(feature = "m3-ci-self-test")]
pub fn ci_self_test() -> Result<(), ObjectError> {
    use crate::memory::heap;

    const REUSE_ITERATIONS: usize = 24_000;

    let heap_before = heap::stats();
    let live_before = counter::live_count();

    let read_only = create_counter(41, Rights::READ)?;
    if counter_read(read_only)? != 41 {
        return Err(ObjectError::SelfTestFailed);
    }

    match counter_increment(read_only) {
        Err(ObjectError::Handle(HandleError::AccessDenied { .. })) => {}
        _ => return Err(ObjectError::SelfTestFailed),
    }

    close(read_only)?;

    if counter::live_count() != live_before || heap::stats() != heap_before {
        return Err(ObjectError::SelfTestFailed);
    }

    match counter_read(read_only) {
        Err(ObjectError::Handle(HandleError::InvalidHandle)) => {}
        _ => return Err(ObjectError::SelfTestFailed),
    }

    let read_write = create_counter(7, Rights::READ | Rights::WRITE)?;

    // The table reuses the first free slot, but the closed handle's generation
    // must never authorize access to the replacement object in that slot.
    if read_write == read_only {
        return Err(ObjectError::SelfTestFailed);
    }
    match counter_read(read_only) {
        Err(ObjectError::Handle(HandleError::InvalidHandle)) => {}
        _ => return Err(ObjectError::SelfTestFailed),
    }

    if counter_increment(read_write)? != 8 || counter_read(read_write)? != 8 {
        return Err(ObjectError::SelfTestFailed);
    }
    close(read_write)?;

    // This loop allocates and destroys far more counter objects than the
    // 512 KiB heap could retain simultaneously if close leaked Arc allocations.
    // Physical frames are never recycled; only blocks inside the mapped heap are.
    for value in 0..REUSE_ITERATIONS as u64 {
        let handle_value = create_counter(value, Rights::READ)?;
        if counter_read(handle_value)? != value {
            return Err(ObjectError::SelfTestFailed);
        }
        close(handle_value)?;
    }

    if counter::live_count() != live_before || heap::stats() != heap_before {
        return Err(ObjectError::SelfTestFailed);
    }

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M3 object self-test: passed reuse_iterations={REUSE_ITERATIONS} live_objects={} live_heap_allocations={}\n",
        counter::live_count(),
        heap::stats().live_allocations,
    ));

    Ok(())
}

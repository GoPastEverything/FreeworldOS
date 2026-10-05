use alloc::sync::Arc;

pub mod counter;
pub mod handle;
pub mod module;
pub mod process;
pub mod task;

use counter::CounterObject;
use handle::{Handle, HandleError, Rights};
use task::{TaskInfo, TaskObject, TaskState};

pub enum FwObject {
    Counter(CounterObject),
    Task(TaskObject),
}

impl FwObject {
    fn can_close_last_handle(&self) -> bool {
        match self {
            Self::Counter(_) => true,
            Self::Task(task) => task.can_release_stack(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectError {
    Handle(HandleError),
    Memory(crate::memory::MemoryError),
    WrongObjectType,
    TaskIdExhausted,
    TaskStackSlotsExhausted,
    SelfTestFailed,
}

impl From<HandleError> for ObjectError {
    fn from(value: HandleError) -> Self {
        Self::Handle(value)
    }
}

impl From<crate::memory::MemoryError> for ObjectError {
    fn from(value: crate::memory::MemoryError) -> Self {
        Self::Memory(value)
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

pub fn create_task(rights: Rights) -> Result<Handle, ObjectError> {
    let object = Arc::new(FwObject::Task(TaskObject::new()?));
    Ok(handle::insert(object, rights)?)
}

pub fn task_info(handle_value: Handle) -> Result<TaskInfo, ObjectError> {
    let object = handle::get(handle_value, Rights::READ)?;

    match object.as_ref() {
        FwObject::Task(task) => Ok(task.info()),
        _ => Err(ObjectError::WrongObjectType),
    }
}

pub fn counter_read(handle_value: Handle) -> Result<u64, ObjectError> {
    let object = handle::get(handle_value, Rights::READ)?;

    match object.as_ref() {
        FwObject::Counter(counter) => Ok(counter.read()),
        _ => Err(ObjectError::WrongObjectType),
    }
}

pub fn counter_increment(handle_value: Handle) -> Result<u64, ObjectError> {
    let object = handle::get(handle_value, Rights::WRITE)?;

    match object.as_ref() {
        FwObject::Counter(counter) => Ok(counter.increment()),
        _ => Err(ObjectError::WrongObjectType),
    }
}

pub fn duplicate(handle_value: Handle, rights: Rights) -> Result<Handle, ObjectError> {
    Ok(handle::duplicate(handle_value, rights)?)
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

    // Basic rights denial and stale-generation proof.
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

    let replacement = create_counter(7, Rights::ALL)?;
    if replacement == read_only {
        return Err(ObjectError::SelfTestFailed);
    }
    match counter_read(read_only) {
        Err(ObjectError::Handle(HandleError::InvalidHandle)) => {}
        _ => return Err(ObjectError::SelfTestFailed),
    }
    close(replacement)?;

    // Capability monotonicity and shared-object lifetime proof.
    let shared = create_counter(10, Rights::ALL)?;
    let reduced = duplicate(shared, Rights::READ)?;

    if counter::live_count() != live_before + 1 {
        return Err(ObjectError::SelfTestFailed);
    }
    if counter_read(reduced)? != 10 {
        return Err(ObjectError::SelfTestFailed);
    }
    match counter_increment(reduced) {
        Err(ObjectError::Handle(HandleError::AccessDenied { .. })) => {}
        _ => return Err(ObjectError::SelfTestFailed),
    }

    // A handle may only mint a duplicate whose rights are a subset of its own.
    match duplicate(reduced, Rights::ALL) {
        Err(ObjectError::Handle(HandleError::AccessDenied { .. })) => {}
        _ => return Err(ObjectError::SelfTestFailed),
    }

    // Closing one of two handles must not destroy the shared object.
    close(shared)?;
    if counter::live_count() != live_before + 1 || counter_read(reduced)? != 10 {
        return Err(ObjectError::SelfTestFailed);
    }

    // Closing the last handle must destroy the object and return heap storage.
    close(reduced)?;
    if counter::live_count() != live_before || heap::stats() != heap_before {
        return Err(ObjectError::SelfTestFailed);
    }

    // Fill the table and prove a failed insertion does not leak the rejected
    // object's Arc while TABLE_LOCK is held.
    let mut full_table = [Handle::INVALID; handle::MAX_HANDLES];
    for (index, slot) in full_table.iter_mut().enumerate() {
        *slot = create_counter(index as u64, Rights::READ)?;
    }

    let live_full = counter::live_count();
    let heap_full = heap::stats();
    match create_counter(0xfeed, Rights::READ) {
        Err(ObjectError::Handle(HandleError::TableFull)) => {}
        _ => return Err(ObjectError::SelfTestFailed),
    }
    if counter::live_count() != live_full || heap::stats() != heap_full {
        return Err(ObjectError::SelfTestFailed);
    }

    for handle_value in full_table {
        close(handle_value)?;
    }

    if counter::live_count() != live_before || heap::stats() != heap_before {
        return Err(ObjectError::SelfTestFailed);
    }

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
        "FreeWorldOS: M3 object self-test: passed reuse_iterations={REUSE_ITERATIONS} duplicate_subset=ok shared_lifetime=ok table_full_drop=ok live_objects={} live_heap_allocations={}\n",
        counter::live_count(),
        heap::stats().live_allocations,
    ));

    Ok(())
}


#[cfg(feature = "m35c-ci-self-test")]
pub fn task_stack_ci_self_test() -> Result<(), ObjectError> {
    let before = crate::memory::frame_reuse_stats()?;
    let task_handle = create_task(Rights::READ)?;
    let info = task_info(task_handle)?;

    if info.id == 0
        || info.state != TaskState::Created
        || info.stack_pages != task::TASK_STACK_PAGES
        || info.guard_page.checked_add(crate::memory::PAGE_SIZE)
            != Some(info.stack_bottom)
        || info.stack_bottom
            .checked_add(task::TASK_STACK_PAGES as u64 * crate::memory::PAGE_SIZE)
            != Some(info.stack_top)
    {
        close(task_handle)?;
        return Err(ObjectError::SelfTestFailed);
    }

    let object = handle::get(task_handle, Rights::READ)?;
    let writable = match object.as_ref() {
        FwObject::Task(task) => task.stack_writable_ci_test(),
        _ => false,
    };
    drop(object);

    if !writable {
        close(task_handle)?;
        return Err(ObjectError::SelfTestFailed);
    }

    close(task_handle)?;

    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total)
        != task::TASK_STACK_PAGES as u64
    {
        return Err(ObjectError::SelfTestFailed);
    }

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M3.5-C task-stack self-test: passed object=ok stack_writable=ok stack_pages={} returned_frames={}\n",
        task::TASK_STACK_PAGES,
        task::TASK_STACK_PAGES,
    ));

    Ok(())
}

#[cfg(feature = "m35c-ci-guard-fault-test")]
pub fn task_guard_fault_ci_test() -> ! {
    let task_handle = create_task(Rights::READ)
        .expect("M3.5-C guard test failed to create task");
    let info = task_info(task_handle)
        .expect("M3.5-C guard test failed to inspect task");

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M3.5-C guard fault test: task_id={} guard={:#x} stack=[{:#x}..{:#x})\n",
        info.id,
        info.guard_page,
        info.stack_bottom,
        info.stack_top,
    ));

    // SAFETY: This dedicated CI image intentionally stores into the reserved
    // not-present page immediately below the task stack. The expected result
    // is the existing fatal #PF diagnostic path; execution must not continue.
    unsafe {
        core::ptr::write_volatile(
            info.guard_page as *mut u64,
            0x4657_4755_4152_4421,
        );
    }

    panic!("M3.5-C guard-page test unexpectedly returned");
}


#[cfg(feature = "m35c2b-ci-self-test")]
pub fn task_ownership_ci_self_test() -> Result<(), ObjectError> {
    use crate::memory::{MemoryError, PagePermissions};

    let task_handle = create_task(Rights::ALL)?;
    let duplicate_handle = duplicate(task_handle, Rights::READ)?;
    let info = task_info(task_handle)?;

    if info.initial_stack_pointer & 0xf != 8
        || info.stack_top & 0xf != 0
        || info.initial_stack_pointer != info.stack_top - 8
    {
        return Err(ObjectError::SelfTestFailed);
    }

    let guard_probe_frame = crate::memory::allocate_frame()?;
    let guard_map = unsafe {
        crate::memory::map_page(
            info.guard_page,
            guard_probe_frame,
            PagePermissions::read_write(),
        )
    };
    if guard_map != Err(MemoryError::VirtualPageReserved) {
        unsafe { crate::memory::free_frame(guard_probe_frame)?; }
        return Err(ObjectError::SelfTestFailed);
    }
    unsafe { crate::memory::free_frame(guard_probe_frame)?; }

    let object = handle::get(task_handle, Rights::READ)?;
    let task = match object.as_ref() {
        FwObject::Task(task) => task,
        _ => return Err(ObjectError::SelfTestFailed),
    };

    task.set_state_for_ci(TaskState::Runnable);
    close(task_handle)?;
    match close(duplicate_handle) {
        Err(ObjectError::Handle(HandleError::ObjectBusy)) => {}
        _ => return Err(ObjectError::SelfTestFailed),
    }

    task.set_state_for_ci(TaskState::Stopped);
    task.set_saved_stack_pointer_present_for_ci(true);
    match close(duplicate_handle) {
        Err(ObjectError::Handle(HandleError::ObjectBusy)) => {}
        _ => return Err(ObjectError::SelfTestFailed),
    }

    task.set_saved_stack_pointer_present_for_ci(false);
    drop(object);
    close(duplicate_handle)?;

    let post_drop_frame = crate::memory::allocate_frame()?;
    unsafe {
        crate::memory::map_page(
            info.guard_page,
            post_drop_frame,
            PagePermissions::read_write(),
        )?;
    }
    let unmapped = crate::memory::unmap_page(info.guard_page)?;
    if unmapped != post_drop_frame {
        unsafe { crate::memory::free_frame(unmapped)?; }
        return Err(ObjectError::SelfTestFailed);
    }
    unsafe { crate::memory::free_frame(unmapped)?; }

    crate::arch::serial::println(
        "FreeWorldOS: M3.5-C ownership self-test: passed runnable_close=denied saved_rsp_close=denied guard_reservation=ok sysv_rsp_align=ok",
    );

    Ok(())
}

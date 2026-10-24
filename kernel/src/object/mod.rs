use alloc::sync::Arc;

pub mod counter;
pub mod handle;
pub mod module;
pub mod process;
#[cfg(feature = "m5m-ci-self-test")]
pub(crate) mod explicit_queue;
pub mod task;

use counter::CounterObject;
use handle::{Handle, HandleError, Rights};
use process::{ProcessInfo, ProcessObject};
use task::{TaskInfo, TaskObject, TaskState};

pub enum FwObject {
    Counter(CounterObject),
    Process(ProcessObject),
    Task(TaskObject),
}

impl FwObject {
    fn can_close_last_handle(&self) -> bool {
        match self {
            Self::Counter(_) | Self::Process(_) => true,
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
    InvalidTaskState,
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
    let object = create_task_ref()?;
    Ok(handle::insert(object, rights)?)
}

pub fn create_process(
    profile: crate::exec::profile::ExecutionProfile,
    rights: Rights,
) -> Result<Handle, ObjectError> {
    let object = Arc::new(FwObject::Process(ProcessObject::new(profile)?));
    Ok(handle::insert(object, rights)?)
}

pub fn create_process_with_user_leaf(
    profile: crate::exec::profile::ExecutionProfile,
    rights: Rights,
    virtual_address: u64,
    permissions: crate::memory::PagePermissions,
) -> Result<Handle, ObjectError> {
    let mut process = ProcessObject::new(profile)?;
    process.map_one_user_leaf(virtual_address, permissions)?;
    // After handle insertion the process is immutable; M5-E deliberately
    // does not expose a concurrent mapping mutation surface.
    let object = Arc::new(FwObject::Process(process));
    Ok(handle::insert(object, rights)?)
}

pub fn process_user_leaf_info(
    handle_value: Handle,
) -> Result<crate::arch::memory::InactiveUserLeafInfo, ObjectError> {
    let object = handle::get(handle_value, Rights::READ)?;
    match object.as_ref() {
        FwObject::Process(process) => Ok(process.inspect_user_leaf()?),
        _ => Err(ObjectError::WrongObjectType),
    }
}

pub(crate) type ObjectRef = Arc<FwObject>;

pub(crate) fn create_task_ref() -> Result<ObjectRef, ObjectError> {
    Ok(Arc::new(FwObject::Task(TaskObject::new()?)))
}

pub(crate) fn install_handle_for_ref(
    object: &ObjectRef,
    rights: Rights,
) -> Result<Handle, ObjectError> {
    Ok(handle::insert(Arc::clone(object), rights)?)
}

pub(crate) fn task_from_ref(object: &ObjectRef) -> Result<&TaskObject, ObjectError> {
    match object.as_ref() {
        FwObject::Task(task) => Ok(task),
        _ => Err(ObjectError::WrongObjectType),
    }
}

pub fn task_info(handle_value: Handle) -> Result<TaskInfo, ObjectError> {
    let object = handle::get(handle_value, Rights::READ)?;

    match object.as_ref() {
        FwObject::Task(task) => Ok(task.info()),
        _ => Err(ObjectError::WrongObjectType),
    }
}

pub fn process_info(handle_value: Handle) -> Result<ProcessInfo, ObjectError> {
    let object = handle::get(handle_value, Rights::READ)?;

    match object.as_ref() {
        FwObject::Process(process) => Ok(process.info()),
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
    match guard_map {
        Err(MemoryError::VirtualPageReserved) => {
            unsafe { crate::memory::free_frame(guard_probe_frame)?; }
        }
        Ok(()) => {
            let mapped = crate::memory::unmap_page(info.guard_page)?;
            unsafe { crate::memory::free_frame(mapped)?; }
            return Err(ObjectError::SelfTestFailed);
        }
        Err(_) => {
            unsafe { crate::memory::free_frame(guard_probe_frame)?; }
            return Err(ObjectError::SelfTestFailed);
        }
    }

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


#[cfg(feature = "m5d-ci-self-test")]
pub fn process_address_space_ci_self_test() -> Result<(), ObjectError> {
    use crate::exec::profile::{
        Abi, Architecture, Environment, ExecutionProfile, ImageFormat,
    };

    let profile = ExecutionProfile {
        environment: Environment::FreeWorld,
        image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64,
        architecture: Architecture::X86_64,
    };

    let live_before = process::live_count();
    let frames_before = crate::memory::frame_reuse_stats()?;

    let first = create_process(profile, Rights::READ)?;
    let second = create_process(profile, Rights::READ)?;

    let first_info = process_info(first)?;
    let second_info = process_info(second)?;

    if first_info.identity.object_id == 0
        || second_info.identity.object_id == 0
        || first_info.identity.object_id == second_info.identity.object_id
        || first_info.identity.profile != profile
        || second_info.identity.profile != profile
        || first_info.address_space_root == second_info.address_space_root
        || process::live_count() != live_before + 2
    {
        return Err(ObjectError::SelfTestFailed);
    }

    let first_object = handle::get(first, Rights::READ)?;
    let second_object = handle::get(second, Rights::READ)?;

    let first_root = match first_object.as_ref() {
        FwObject::Process(process) => process.inspect_address_space()?,
        _ => return Err(ObjectError::WrongObjectType),
    };
    let second_root = match second_object.as_ref() {
        FwObject::Process(process) => process.inspect_address_space()?,
        _ => return Err(ObjectError::WrongObjectType),
    };

    if first_root.root_frame != first_info.address_space_root
        || second_root.root_frame != second_info.address_space_root
        || first_root.active_kernel_root_frame != second_root.active_kernel_root_frame
        || first_root.root_frame == first_root.active_kernel_root_frame
        || second_root.root_frame == second_root.active_kernel_root_frame
        || !first_root.lower_half_empty
        || !second_root.lower_half_empty
        || !first_root.higher_half_matches_kernel
        || !second_root.higher_half_matches_kernel
        || first_root.higher_half_user_accessible
        || second_root.higher_half_user_accessible
    {
        return Err(ObjectError::SelfTestFailed);
    }

    let kernel_root = first_root.active_kernel_root_frame;

    drop(first_object);
    drop(second_object);
    close(first)?;
    close(second)?;

    if process::live_count() != live_before {
        return Err(ObjectError::SelfTestFailed);
    }

    let frames_after = crate::memory::frame_reuse_stats()?;
    if frames_after.returned_total.saturating_sub(frames_before.returned_total) != 2 {
        return Err(ObjectError::SelfTestFailed);
    }

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M5-D process roots: first={:#x} second={:#x} kernel={:#x} distinct=ok lower_private_empty=ok higher_shared=ok higher_user=off\n",
        first_info.address_space_root.start,
        second_info.address_space_root.start,
        kernel_root.start,
    ));
    crate::arch::serial::println(
        "FreeWorldOS: M5-D process address-space self-test: passed process_object=ok own_pml4=ok lower_half=private_empty higher_half=kernel_shared root_reclaim=2 cr3_switch=off user_execution=off preemption=off callgate=off",
    );

    Ok(())
}


#[cfg(feature = "m5e-ci-self-test")]
pub fn process_user_leaf_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::{MemoryError, PagePermissions},
    };

    const USER_VA: u64 = 0x0000_5000_2000_0000;
    const PATTERN: u64 = 0x4d35_4555_5345_524c;

    let profile = ExecutionProfile {
        environment: Environment::FreeWorld,
        image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64,
        architecture: Architecture::X86_64,
    };

    // A rejected out-of-range mapping must not publish a process handle or
    // leave behind its newly allocated empty PML4 root.
    if !matches!(
        create_process_with_user_leaf(
            profile, Rights::READ,
            0xffff_e000_0000_0000,
            PagePermissions::user_read_write(),
        ),
        Err(ObjectError::Memory(MemoryError::UserMappingOutsideLowerHalf))
    ) {
        return Err(ObjectError::SelfTestFailed);
    }

    let live_before = process::live_count();
    let before = crate::memory::frame_reuse_stats()?;
    let mapped = create_process_with_user_leaf(
        profile, Rights::READ, USER_VA, PagePermissions::user_read_write(),
    )?;
    let empty_peer = create_process(profile, Rights::READ)?;

    let mapped_info = process_info(mapped)?;
    let empty_info = process_info(empty_peer)?;
    let leaf = process_user_leaf_info(mapped)?;

    let mapped_ref = handle::get(mapped, Rights::READ)?;
    let empty_ref = handle::get(empty_peer, Rights::READ)?;

    let (mapped_root, content_ok) = match mapped_ref.as_ref() {
        FwObject::Process(p) => (
            p.inspect_address_space()?,
            p.ci_probe_user_leaf(PATTERN)?,
        ),
        _ => return Err(ObjectError::WrongObjectType),
    };
    let empty_root = match empty_ref.as_ref() {
        FwObject::Process(p) => p.inspect_address_space()?,
        _ => return Err(ObjectError::WrongObjectType),
    };

    if mapped_info.address_space_root == empty_info.address_space_root
        || leaf.root_frame != mapped_info.address_space_root
        || leaf.virtual_address != USER_VA
        || leaf.data_frame == mapped_info.address_space_root
        || !leaf.ancestor_user_accessible
        || !leaf.leaf_user_accessible
        || !leaf.leaf_writable
        || !leaf.leaf_non_executable
        || !content_ok
        || mapped_root.lower_half_empty
        || !empty_root.lower_half_empty
        || !mapped_root.higher_half_matches_kernel
        || !empty_root.higher_half_matches_kernel
        || mapped_root.higher_half_user_accessible
        || empty_root.higher_half_user_accessible
        || mapped_root.active_kernel_root_frame != empty_root.active_kernel_root_frame
        || mapped_root.active_kernel_root_frame == mapped_info.address_space_root
        || empty_root.active_kernel_root_frame == empty_info.address_space_root
        || process::live_count() != live_before + 2
    {
        return Err(ObjectError::SelfTestFailed);
    }

    // These handle references must go away before final-close reclamation.
    drop(mapped_ref);
    drop(empty_ref);
    close(mapped)?;
    close(empty_peer)?;

    if process::live_count() != live_before {
        return Err(ObjectError::SelfTestFailed);
    }

    let after = crate::memory::frame_reuse_stats()?;
    // Mapped process: 1 PML4 + 3 private tables + 1 data leaf = 5 frames.
    // Empty peer: 1 PML4 = 1 frame. No kernel-half frame is released.
    if after.returned_total.saturating_sub(before.returned_total) != 6 {
        return Err(ObjectError::SelfTestFailed);
    }

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M5-E inactive leaf: root={:#x} peer={:#x} virtual={:#x} data={:#x} ancestors=user leaf=rw_nx peer_lower=empty kernel_half=shared active_cr3=unchanged\n",
        mapped_info.address_space_root.start,
        empty_info.address_space_root.start,
        USER_VA,
        leaf.data_frame.start,
    ));
    crate::arch::serial::println(
        "FreeWorldOS: M5-E process user-leaf self-test: passed inactive_mapping=ok user_leaf=1 ancestors=3 user_writable=ok leaf_nx=ok isolated_peer=ok rollback_rejected=ok frames_returned=6 cr3_switch=off user_execution=off callgate=off",
    );

    Ok(())
}


#[cfg(feature = "m5f-ci-self-test")]
pub fn process_cr3_roundtrip_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };

    const USER_VA: u64 = 0x0000_5000_2000_0000;
    const PATTERN: u64 = 0x4d35_4650_524f_4345;

    let profile = ExecutionProfile {
        environment: Environment::FreeWorld,
        image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64,
        architecture: Architecture::X86_64,
    };

    let live_before = process::live_count();
    let before = crate::memory::frame_reuse_stats()?;

    let mapped = create_process_with_user_leaf(
        profile, Rights::READ, USER_VA, PagePermissions::user_read_write(),
    )?;
    let empty_peer = create_process(profile, Rights::READ)?;
    let mapped_info = process_info(mapped)?;
    let empty_info = process_info(empty_peer)?;

    // Hold a strong object reference for the entire switch. Closing a handle
    // cannot reclaim the active process root while it is being probed.
    let mapped_ref = handle::get(mapped, Rights::READ)?;
    let peer_ref = handle::get(empty_peer, Rights::READ)?;
    let proof = match mapped_ref.as_ref() {
        FwObject::Process(p) => p.ci_cr3_roundtrip(PATTERN)?,
        _ => return Err(ObjectError::WrongObjectType),
    };
    let peer = match peer_ref.as_ref() {
        FwObject::Process(p) => p.inspect_address_space()?,
        _ => return Err(ObjectError::WrongObjectType),
    };

    if proof.kernel_cr3_before != proof.kernel_cr3_after
        || proof.process_cr3_observed != mapped_info.address_space_root.start
        || proof.kernel_cr3_before == proof.process_cr3_observed
        || proof.virtual_readback != PATTERN
        || proof.physical_readback != PATTERN
        || !proof.absent_from_kernel_root
        || !peer.lower_half_empty
        || !peer.higher_half_matches_kernel
        || peer.higher_half_user_accessible
        || peer.root_frame != empty_info.address_space_root
        || peer.root_frame == mapped_info.address_space_root
        || process::live_count() != live_before + 2
    {
        return Err(ObjectError::SelfTestFailed);
    }

    drop(mapped_ref);
    drop(peer_ref);
    close(mapped)?;
    close(empty_peer)?;

    if process::live_count() != live_before {
        return Err(ObjectError::SelfTestFailed);
    }
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 6 {
        return Err(ObjectError::SelfTestFailed);
    }

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M5-F CR3 roundtrip: kernel_before={:#x} process={:#x} kernel_after={:#x} virtual={USER_VA:#x} value={PATTERN:#x} peer_root={:#x} kernel_leaf_absent=ok\n",
        proof.kernel_cr3_before,
        proof.process_cr3_observed,
        proof.kernel_cr3_after,
        peer.root_frame.start,
    ));
    crate::arch::serial::println(
        "FreeWorldOS: M5-F process CR3 self-test: passed process_cr3=loaded user_virtual_rw=ok kernel_cr3=restored kernel_root_leaf=absent physical_direct_map=match peer_isolated=ok frames_returned=6 if=masked cpl0=only preemption=off callgate=off",
    );
    Ok(())
}


#[cfg(feature = "m5g-ci-self-test")]
pub fn process_ring3_cr3_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };

    const USER_CODE_VA: u64 = 0x0000_5000_3000_0000;
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld,
        image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64,
        architecture: Architecture::X86_64,
    };
    let live_before = process::live_count();
    let before = crate::memory::frame_reuse_stats()?;

    // M5-E's one-leaf process has only an executable code page. This stub
    // makes NO user-stack writes. A real writable user stack is deferred.
    let mapped = create_process_with_user_leaf(
        profile, Rights::READ, USER_CODE_VA, PagePermissions::user_read_execute(),
    )?;
    let empty_peer = create_process(profile, Rights::READ)?;
    let mapped_info = process_info(mapped)?;
    let empty_info = process_info(empty_peer)?;
    let mapped_ref = handle::get(mapped, Rights::READ)?;
    let peer_ref = handle::get(empty_peer, Rights::READ)?;

    let (proof, user_leaf) = match mapped_ref.as_ref() {
        FwObject::Process(p) => (
            p.ci_ring3_process_roundtrip()?,
            p.inspect_user_leaf()?,
        ),
        _ => return Err(ObjectError::WrongObjectType),
    };
    let peer_info = match peer_ref.as_ref() {
        FwObject::Process(p) => p.inspect_address_space()?,
        _ => return Err(ObjectError::WrongObjectType),
    };

    if proof.observed_process_cr3 & !0xfff != mapped_info.address_space_root.start
        || proof.original_kernel_cr3 != proof.restored_kernel_cr3
        || proof.original_kernel_cr3 == mapped_info.address_space_root.start
        || proof.frame_address == 0
        || !user_leaf.leaf_user_accessible
        || user_leaf.leaf_writable
        || user_leaf.leaf_non_executable
        || !user_leaf.ancestor_user_accessible
        || user_leaf.virtual_address != USER_CODE_VA
        || !peer_info.lower_half_empty
        || !peer_info.higher_half_matches_kernel
        || peer_info.higher_half_user_accessible
        || peer_info.root_frame != empty_info.address_space_root
        || peer_info.root_frame == mapped_info.address_space_root
        || process::live_count() != live_before + 2
    {
        return Err(ObjectError::SelfTestFailed);
    }

    drop(mapped_ref);
    drop(peer_ref);
    close(mapped)?;
    close(empty_peer)?;
    if process::live_count() != live_before {
        return Err(ObjectError::SelfTestFailed);
    }
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 6 {
        return Err(ObjectError::SelfTestFailed);
    }

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M5-G process ring3: kernel_before={:#x} process={:#x} kernel_after={:#x} code={USER_CODE_VA:#x} frame={:#x} cs=0x23 ss=0x1b vector=0xf2\n",
        proof.original_kernel_cr3,
        proof.observed_process_cr3 & !0xfff,
        proof.restored_kernel_cr3,
        proof.frame_address,
    ));
    crate::arch::serial::println(
        "FreeWorldOS: M5-G process ring3 self-test: passed process_cr3=active_in_cpl3 iretq=ok user_rx=ok user_stack_writes=off tss_rsp0=used kernel_cr3=restored_before_rust frame_reclaim=6 if=masked scheduler=off callgate=off",
    );
    Ok(())
}

#[cfg(feature = "m5h-ci-self-test")]
pub fn process_user_stack_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    const CODE: u64 = 0x0000_5000_3000_0000;
    const STACK: u64 = 0x0000_5000_3000_1000;
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld,
        image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64,
        architecture: Architecture::X86_64,
    };
    let live_before = process::live_count();
    let before = crate::memory::frame_reuse_stats()?;
    // Both mappings are installed before the immutable process is published.
    let mut process = ProcessObject::new(profile)?;
    process.map_one_user_leaf(CODE, PagePermissions::user_read_execute())?;
    process.map_adjacent_user_stack()?;
    let mapped = handle::insert(Arc::new(FwObject::Process(process)), Rights::READ)?;
    let empty_peer = create_process(profile, Rights::READ)?;
    let mapped_info = process_info(mapped)?;
    let empty_info = process_info(empty_peer)?;
    let mapped_ref = handle::get(mapped, Rights::READ)?;
    let peer_ref = handle::get(empty_peer, Rights::READ)?;
    let (proof, code) = match mapped_ref.as_ref() {
        FwObject::Process(p) => (p.ci_stack_push()?, p.inspect_user_leaf()?),
        _ => return Err(ObjectError::WrongObjectType),
    };
    let peer = match peer_ref.as_ref() {
        FwObject::Process(p) => p.inspect_address_space()?,
        _ => return Err(ObjectError::WrongObjectType),
    };
    if proof.observed_process_cr3 & !0xfff != mapped_info.address_space_root.start
        || proof.original_kernel_cr3 != proof.restored_kernel_cr3
        || proof.frame_address == 0
        || code.virtual_address != CODE
        || !code.ancestor_user_accessible
        || !code.leaf_user_accessible
        || code.leaf_writable
        || code.leaf_non_executable
        || !peer.lower_half_empty
        || !peer.higher_half_matches_kernel
        || peer.higher_half_user_accessible
        || peer.root_frame != empty_info.address_space_root
        || peer.root_frame == mapped_info.address_space_root
        || process::live_count() != live_before + 2
    {
        return Err(ObjectError::SelfTestFailed);
    }
    drop(mapped_ref);
    drop(peer_ref);
    close(mapped)?;
    close(empty_peer)?;
    if process::live_count() != live_before {
        return Err(ObjectError::SelfTestFailed);
    }
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 7 {
        return Err(ObjectError::SelfTestFailed);
    }
    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M5-H process stack: kernel_before={:#x} process={:#x} kernel_after={:#x} code={CODE:#x} stack={STACK:#x} entry_rsp=0x500030002000 pushed_rsp=0x500030001ff8 ud2=0x50003000000d frames_returned=7\n",
        proof.original_kernel_cr3,
        proof.observed_process_cr3 & !0xfff,
        proof.restored_kernel_cr3,
    ));
    crate::arch::serial::println(
        "FreeWorldOS: M5-H user stack self-test: passed code=rx_clean stack=rw_nx_dirty_allowed marker=direct_map_after_cr3_restored shared_ancestors=3 frames_returned=7 rsp0=shared if=masked scheduler=off callgate=off",
    );
    Ok(())
}

#[cfg(feature = "m5i-ci-self-test")]
pub fn process_private_rsp0_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    const CODE: u64 = 0x0000_5000_3000_0000;
    const STACK: u64 = 0x0000_5000_3000_1000;
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld,
        image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64,
        architecture: Architecture::X86_64,
    };
    let live_before = process::live_count();
    let before = crate::memory::frame_reuse_stats()?;
    // Both mappings are installed before the immutable process is published.
    let mut process = ProcessObject::new_with_private_kernel_stack(profile)?;
    process.map_one_user_leaf(CODE, PagePermissions::user_read_execute())?;
    process.map_adjacent_user_stack()?;
    let mapped = handle::insert(Arc::new(FwObject::Process(process)), Rights::READ)?;
    let empty_peer = create_process(profile, Rights::READ)?;
    let mapped_info = process_info(mapped)?;
    let empty_info = process_info(empty_peer)?;
    let mapped_ref = handle::get(mapped, Rights::READ)?;
    let peer_ref = handle::get(empty_peer, Rights::READ)?;
    let (proof, code) = match mapped_ref.as_ref() {
        FwObject::Process(p) => (p.ci_private_rsp0()?, p.inspect_user_leaf()?),
        _ => return Err(ObjectError::WrongObjectType),
    };
    let peer = match peer_ref.as_ref() {
        FwObject::Process(p) => p.inspect_address_space()?,
        _ => return Err(ObjectError::WrongObjectType),
    };
    if proof.observed_process_cr3 & !0xfff != mapped_info.address_space_root.start
        || proof.original_kernel_cr3 != proof.restored_kernel_cr3
        || proof.frame_address == 0
        || code.virtual_address != CODE
        || !code.ancestor_user_accessible
        || !code.leaf_user_accessible
        || code.leaf_writable
        || code.leaf_non_executable
        || !peer.lower_half_empty
        || !peer.higher_half_matches_kernel
        || peer.higher_half_user_accessible
        || peer.root_frame != empty_info.address_space_root
        || peer.root_frame == mapped_info.address_space_root
        || process::live_count() != live_before + 2
    {
        return Err(ObjectError::SelfTestFailed);
    }
    drop(mapped_ref);
    drop(peer_ref);
    close(mapped)?;
    close(empty_peer)?;
    if process::live_count() != live_before {
        return Err(ObjectError::SelfTestFailed);
    }
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 11 {
        return Err(ObjectError::SelfTestFailed);
    }
    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M5-I process stack: kernel_before={:#x} process={:#x} kernel_after={:#x} code={CODE:#x} stack={STACK:#x} entry_rsp=0x500030002000 pushed_rsp=0x500030001ff8 ud2=0x50003000000d frames_returned=11\n",
        proof.original_kernel_cr3,
        proof.observed_process_cr3 & !0xfff,
        proof.restored_kernel_cr3,
    ));
    crate::arch::serial::println(
        "FreeWorldOS: M5-I private RSP0 self-test: passed frame=160 private_rsp0=used original_rsp0=restored process_owned=ok frames_returned=11 if=masked scheduler=off callgate=off",
    );
    Ok(())
}

#[cfg(feature = "m5j-ci-self-test")]
pub fn two_process_private_rsp0_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    const CODE: u64 = 0x0000_5000_3000_0000;
    const FRAME_BYTES: u64 = 160;

    let profile = ExecutionProfile {
        environment: Environment::FreeWorld,
        image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64,
        architecture: Architecture::X86_64,
    };
    let live_before = process::live_count();
    let before = crate::memory::frame_reuse_stats()?;
    let bootstrap_rsp0 = crate::arch::current_tss_rsp0();
    if bootstrap_rsp0 != crate::arch::ring0_privilege_stack_top() {
        return Err(ObjectError::SelfTestFailed);
    }

    // Two distinct ProcessObjects; no user TaskObjects or runnable queue.
    // Map each private kernel stack BEFORE creating its PML4.
    let mut a = ProcessObject::new_with_private_kernel_stack(profile)?;
    a.map_one_user_leaf(CODE, PagePermissions::user_read_execute())?;
    a.map_adjacent_user_stack()?;
    let mut b = ProcessObject::new_with_private_kernel_stack(profile)?;
    b.map_one_user_leaf(CODE, PagePermissions::user_read_execute())?;
    b.map_adjacent_user_stack()?;
    let handle_a = handle::insert(Arc::new(FwObject::Process(a)), Rights::READ)?;
    let handle_b = handle::insert(Arc::new(FwObject::Process(b)), Rights::READ)?;
    let ref_a = handle::get(handle_a, Rights::READ)?;
    let ref_b = handle::get(handle_b, Rights::READ)?;

    let (root_a, bounds_a, leaf_a) = match ref_a.as_ref() {
        FwObject::Process(p) => (
            p.info().address_space_root,
            p.ci_private_stack_bounds()?,
            p.inspect_user_leaf()?,
        ),
        _ => return Err(ObjectError::WrongObjectType),
    };
    let (root_b, bounds_b, leaf_b) = match ref_b.as_ref() {
        FwObject::Process(p) => (
            p.info().address_space_root,
            p.ci_private_stack_bounds()?,
            p.inspect_user_leaf()?,
        ),
        _ => return Err(ObjectError::WrongObjectType),
    };

    if root_a == root_b
        || leaf_a.data_frame == leaf_b.data_frame
        || bounds_a.0 >= bounds_a.1
        || bounds_b.0 >= bounds_b.1
        || bounds_a.0 < bounds_b.1 && bounds_b.0 < bounds_a.1
        || leaf_a.virtual_address != CODE || leaf_b.virtual_address != CODE
        || !leaf_a.leaf_user_accessible || !leaf_b.leaf_user_accessible
        || leaf_a.leaf_writable || leaf_b.leaf_writable
        || leaf_a.leaf_non_executable || leaf_b.leaf_non_executable
        || process::live_count() != live_before + 2
    {
        return Err(ObjectError::SelfTestFailed);
    }

    // A: one masked 0xF2 entry, CR3 restored before Rust, then RSP0 restored.
    let proof_a = match ref_a.as_ref() {
        FwObject::Process(p) => p.ci_private_rsp0()?,
        _ => return Err(ObjectError::WrongObjectType),
    };
    if crate::arch::current_tss_rsp0() != bootstrap_rsp0
        || proof_a.observed_process_cr3 & !0xfff != root_a.start
        || proof_a.original_kernel_cr3 != proof_a.restored_kernel_cr3
        || proof_a.frame_address < bounds_a.0
        || proof_a.frame_address.checked_add(FRAME_BYTES).is_none_or(|end| end > bounds_a.1)
        || proof_a.frame_address >= bounds_b.0 && proof_a.frame_address < bounds_b.1
    {
        return Err(ObjectError::SelfTestFailed);
    }

    // B: explicitly sequential, after A has completely returned.
    let proof_b = match ref_b.as_ref() {
        FwObject::Process(p) => p.ci_private_rsp0()?,
        _ => return Err(ObjectError::WrongObjectType),
    };
    if crate::arch::current_tss_rsp0() != bootstrap_rsp0
        || proof_b.observed_process_cr3 & !0xfff != root_b.start
        || proof_b.original_kernel_cr3 != proof_b.restored_kernel_cr3
        || proof_b.original_kernel_cr3 != proof_a.original_kernel_cr3
        || proof_b.frame_address < bounds_b.0
        || proof_b.frame_address.checked_add(FRAME_BYTES).is_none_or(|end| end > bounds_b.1)
        || proof_b.frame_address >= bounds_a.0 && proof_b.frame_address < bounds_a.1
    {
        return Err(ObjectError::SelfTestFailed);
    }

    // Keep strong references alive until both proof intervals have ended.
    drop(ref_a);
    drop(ref_b);
    close(handle_a)?;
    close(handle_b)?;
    if process::live_count() != live_before
        || crate::arch::current_tss_rsp0() != bootstrap_rsp0
    {
        return Err(ObjectError::SelfTestFailed);
    }
    let after = crate::memory::frame_reuse_stats()?;
    // Per process: four private kernel-stack leaves + root + two user
    // leaves + three user page-table ancestors = ten frames.
    if after.returned_total.saturating_sub(before.returned_total) != 20 {
        return Err(ObjectError::SelfTestFailed);
    }

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M5-J sequential process entry: root_a={:#x} root_b={:#x} frame_a={:#x} frame_b={:#x} stack_a=[{:#x}..{:#x}) stack_b=[{:#x}..{:#x}) bootstrap_rsp0={bootstrap_rsp0:#x} restored_each=ok frames_returned=20\n",
        root_a.start, root_b.start, proof_a.frame_address, proof_b.frame_address,
        bounds_a.0, bounds_a.1, bounds_b.0, bounds_b.1
    ));
    crate::arch::serial::println(
        "FreeWorldOS: M5-J two-process private RSP0 self-test: passed distinct_cr3=ok distinct_kernel_stacks=ok frame_a=private_160 frame_b=private_160 bootstrap_rsp0=restored_after_each sequential=ok frames_returned=20 if=masked timer_switch=off scheduler=off callgate=off",
    );
    Ok(())
}

#[cfg(feature = "m5k-ci-self-test")]
pub fn process_suspend_resume_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    const CODE: u64 = 0x0000_5000_3000_0000;
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld,
        image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64,
        architecture: Architecture::X86_64,
    };
    let original_rsp0 = crate::arch::current_tss_rsp0();
    if original_rsp0 != crate::arch::ring0_privilege_stack_top() {
        return Err(ObjectError::SelfTestFailed);
    }
    let live_before = process::live_count();
    let before = crate::memory::frame_reuse_stats()?;
    // Kernel stack mapped first, then copied into the process-owned PML4.
    let mut a = ProcessObject::new_with_private_kernel_stack(profile)?;
    a.map_one_user_leaf(CODE, PagePermissions::user_read_execute())?;
    a.map_adjacent_user_stack()?;
    let handle_a = handle::insert(Arc::new(FwObject::Process(a)), Rights::READ)?;
    let strong_a = handle::get(handle_a, Rights::READ)?;
    let (proof, stack, root) = match strong_a.as_ref() {
        FwObject::Process(p) => {
            let bounds = p.ci_private_stack_bounds()?;
            let root = p.info().address_space_root.start;
            (p.ci_suspend_resume()?, bounds, root)
        },
        _ => return Err(ObjectError::WrongObjectType),
    };
    if proof.observed_process_cr3 & !0xfff != root
        || proof.original_kernel_cr3 != proof.restored_kernel_cr3
        || proof.frame_address != stack.1 - 160
        || stack.1 - stack.0 != 4 * 4096
        || crate::arch::current_tss_rsp0() != original_rsp0
        || process::live_count() != live_before + 1
    {
        return Err(ObjectError::SelfTestFailed);
    }
    drop(strong_a);
    close(handle_a)?;
    if process::live_count() != live_before || crate::arch::current_tss_rsp0() != original_rsp0 {
        return Err(ObjectError::SelfTestFailed);
    }
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 10 {
        return Err(ObjectError::SelfTestFailed);
    }
    crate::arch::serial::println(
        "FreeWorldOS: M5-K suspend/resume self-test: passed first_frame=top_minus_160 second_frame=top_minus_320 resume_rip=0x50003000000d final_rip=0x50003000000f marker=once final_cr3=kernel bootstrap_rsp0=restored_after_final frames_returned=10 if=masked scheduler=off callgate=off",
    );
    Ok(())
}

#[cfg(feature = "m5l-ci-self-test")]
pub fn suspend_a_run_b_resume_a_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    const CODE: u64 = 0x0000_5000_3000_0000;
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld,
        image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64,
        architecture: Architecture::X86_64,
    };
    let bootstrap_rsp0 = crate::arch::current_tss_rsp0();
    let before = crate::memory::frame_reuse_stats()?;
    let live_before = process::live_count();
    if bootstrap_rsp0 != crate::arch::ring0_privilege_stack_top() {
        return Err(ObjectError::SelfTestFailed);
    }
    // Both kernel stack mappings exist before the respective root's
    // kernel-half snapshot; neither process is ever placed in a run queue.
    let mut a = ProcessObject::new_with_private_kernel_stack(profile)?;
    a.map_one_user_leaf(CODE, PagePermissions::user_read_execute())?;
    a.map_adjacent_user_stack()?;
    let mut b = ProcessObject::new_with_private_kernel_stack(profile)?;
    b.map_one_user_leaf(CODE, PagePermissions::user_read_execute())?;
    b.map_adjacent_user_stack()?;
    let handle_a = handle::insert(Arc::new(FwObject::Process(a)), Rights::READ)?;
    let handle_b = handle::insert(Arc::new(FwObject::Process(b)), Rights::READ)?;
    let ref_a = handle::get(handle_a, Rights::READ)?;
    let ref_b = handle::get(handle_b, Rights::READ)?;
    let (root_a, bounds_a, code_a) = match ref_a.as_ref() {
        FwObject::Process(p) => (
            p.info().address_space_root.start,
            p.ci_private_stack_bounds()?,
            p.inspect_user_leaf()?,
        ),
        _ => return Err(ObjectError::WrongObjectType),
    };
    let (root_b, bounds_b, code_b) = match ref_b.as_ref() {
        FwObject::Process(p) => (
            p.info().address_space_root.start,
            p.ci_private_stack_bounds()?,
            p.inspect_user_leaf()?,
        ),
        _ => return Err(ObjectError::WrongObjectType),
    };
    if root_a == root_b || code_a.data_frame == code_b.data_frame
        || bounds_a.0 < bounds_b.1 && bounds_b.0 < bounds_a.1
        || bounds_a.1 - bounds_a.0 != 4 * 4096
        || bounds_b.1 - bounds_b.0 != 4 * 4096
    {
        return Err(ObjectError::SelfTestFailed);
    }
    // B runs fully while A's original 160-byte frame remains resident.
    // The return-gate snapshots and verifies all 20 words across B.
    let mut observed_b = None;
    let proof_a = match ref_a.as_ref() {
        FwObject::Process(p) => p.ci_suspend_a_b_a(|| {
            observed_b = Some(match ref_b.as_ref() {
                FwObject::Process(b) => b.ci_private_rsp0_as_b(bounds_a.1),
                _ => unreachable!(),
            });
        })?,
        _ => return Err(ObjectError::WrongObjectType),
    };
    let proof_b = observed_b.ok_or(ObjectError::SelfTestFailed)??;
    if proof_a.observed_process_cr3 & !0xfff != root_a
        || proof_b.observed_process_cr3 & !0xfff != root_b
        || proof_a.original_kernel_cr3 != proof_a.restored_kernel_cr3
        || proof_b.original_kernel_cr3 != proof_b.restored_kernel_cr3
        || proof_a.original_kernel_cr3 != proof_b.original_kernel_cr3
        || proof_a.frame_address != bounds_a.1 - 160
        || proof_b.frame_address != bounds_b.1 - 160
        || proof_b.frame_address >= bounds_a.0 && proof_b.frame_address < bounds_a.1
        || crate::arch::current_tss_rsp0() != bootstrap_rsp0
        || process::live_count() != live_before + 2
    {
        return Err(ObjectError::SelfTestFailed);
    }
    drop(ref_a);
    drop(ref_b);
    close(handle_a)?;
    close(handle_b)?;
    if process::live_count() != live_before || crate::arch::current_tss_rsp0() != bootstrap_rsp0 {
        return Err(ObjectError::SelfTestFailed);
    }
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 20 {
        return Err(ObjectError::SelfTestFailed);
    }
    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M5-L isolation: root_a={root_a:#x} root_b={root_b:#x} frame_a={:#x} frame_b={:#x} a_top={:#x} b_top={:#x} saved_frame_unchanged=yes rsp0=restored frames_returned=20\n",
        proof_a.frame_address, proof_b.frame_address, bounds_a.1, bounds_b.1,
    ));
    crate::arch::serial::println(
        "FreeWorldOS: M5-L A-B-A isolation self-test: passed a_frame=unchanged_after_b a_resume_rip=0x50003000000d b_frame=private_only cr3=distinct marker=once rsp0=restored_after_final frames_returned=20 if=masked scheduler=off callgate=off",
    );
    Ok(())
}

#[cfg(feature = "m5m-ci-self-test")]
pub fn explicit_two_slot_queue_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    const CODE: u64 = 0x0000_5000_3000_0000;
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld, image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64, architecture: Architecture::X86_64,
    };
    explicit_queue::ci_self_test().map_err(|_| ObjectError::SelfTestFailed)?;
    let bootstrap_rsp0 = crate::arch::current_tss_rsp0();
    let before = crate::memory::frame_reuse_stats()?;
    let live_before = process::live_count();
    if bootstrap_rsp0 != crate::arch::ring0_privilege_stack_top() {
        return Err(ObjectError::SelfTestFailed);
    }
    let mut a = ProcessObject::new_with_private_kernel_stack(profile)?;
    a.map_one_user_leaf(CODE, PagePermissions::user_read_execute())?;
    a.map_adjacent_user_stack()?;
    let mut b = ProcessObject::new_with_private_kernel_stack(profile)?;
    b.map_one_user_leaf(CODE, PagePermissions::user_read_execute())?;
    b.map_adjacent_user_stack()?;
    let handle_a = handle::insert(Arc::new(FwObject::Process(a)), Rights::READ)?;
    let handle_b = handle::insert(Arc::new(FwObject::Process(b)), Rights::READ)?;
    let ref_a = handle::get(handle_a, Rights::READ)?;
    let ref_b = handle::get(handle_b, Rights::READ)?;
    let (root_a, bounds_a, code_a) = match ref_a.as_ref() {
        FwObject::Process(p) => {
            p.ci_stage_explicit_queue(true)?;
            (p.info().address_space_root.start, p.ci_private_stack_bounds()?,
                p.inspect_user_leaf()?)
        },
        _ => return Err(ObjectError::WrongObjectType),
    };
    let (root_b, bounds_b, code_b) = match ref_b.as_ref() {
        FwObject::Process(p) => {
            p.ci_stage_explicit_queue(false)?;
            (p.info().address_space_root.start, p.ci_private_stack_bounds()?,
                p.inspect_user_leaf()?)
        },
        _ => return Err(ObjectError::WrongObjectType),
    };
    if root_a == root_b || code_a.data_frame == code_b.data_frame
        || bounds_a.0 < bounds_b.1 && bounds_b.0 < bounds_a.1
        || bounds_a.1 - bounds_a.0 != 4 * 4096
        || bounds_b.1 - bounds_b.0 != 4 * 4096
        || process::live_count() != live_before + 2 {
        return Err(ObjectError::SelfTestFailed);
    }

    let proof = unsafe { crate::arch::process_ring3::run_explicit_two_slot_queue(
        root_a, bounds_a.0, bounds_a.1, root_b, bounds_b.0, bounds_b.1,
    ) };
    if proof.root_a != root_a || proof.root_b != root_b
        || proof.frame_a_first != bounds_a.1 - 160
        || proof.frame_a_second != bounds_a.1 - 320
        || proof.frame_a_final != bounds_a.1 - 480
        || proof.frame_b_first != bounds_b.1 - 160
        || proof.frame_b_second != bounds_b.1 - 320
        || crate::arch::current_tss_rsp0() != bootstrap_rsp0 {
        return Err(ObjectError::SelfTestFailed);
    }
    // Read each pushed marker once, after both CR3 resumes finished.
    match ref_a.as_ref() {
        FwObject::Process(p) => p.ci_check_explicit_queue_marker()?,
        _ => return Err(ObjectError::WrongObjectType),
    }
    match ref_b.as_ref() {
        FwObject::Process(p) => p.ci_check_explicit_queue_marker()?,
        _ => return Err(ObjectError::WrongObjectType),
    }
    drop(ref_a); drop(ref_b);
    close(handle_a)?; close(handle_b)?;
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 20
        || process::live_count() != live_before
        || crate::arch::current_tss_rsp0() != bootstrap_rsp0 {
        return Err(ObjectError::SelfTestFailed);
    }
    crate::arch::serial::println(
        "FreeWorldOS: M5-M two-slot explicit queue: passed picks=A,B,A a_frames=three_private b_frames=two_private distinct_cr3=ok markers=once bootstrap_rsp0=restored_at_end frames_returned=20 if=masked timer_choice=off callgate=off",
    );
    Ok(())
}

#[cfg(feature = "m5n-ci-self-test")]
pub fn test_kernel_owned_callgate_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld, image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64, architecture: Architecture::X86_64,
    };
    let bootstrap = crate::arch::current_tss_rsp0();
    let before = crate::memory::frame_reuse_stats()?;
    let live_before = process::live_count();
    let mut process = ProcessObject::new_with_private_kernel_stack(profile)?;
    process.map_one_user_leaf(0x0000_5000_3000_0000, PagePermissions::user_read_execute())?;
    process.map_adjacent_user_stack()?;
    process.map_test_callgate()?;
    let handle = handle::insert(Arc::new(FwObject::Process(process)), Rights::READ)?;
    let strong = handle::get(handle, Rights::READ)?;
    let proof = match strong.as_ref() {
        FwObject::Process(p) => p.ci_callgate_proof()?,
        _ => return Err(ObjectError::WrongObjectType),
    };
    if proof.original_kernel_cr3 != proof.restored_kernel_cr3
        || proof.original_kernel_cr3 == proof.observed_process_cr3
        || crate::arch::current_tss_rsp0() != bootstrap
        || process::live_count() != live_before + 1 {
        return Err(ObjectError::SelfTestFailed);
    }
    drop(strong);
    close(handle)?;
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 11
        || process::live_count() != live_before {
        return Err(ObjectError::SelfTestFailed);
    }
    crate::arch::serial::println(
        "FreeWorldOS: M5-N callgate self-test: passed gate=user_rx_kernel_owned call=near_return first=int_f2_from_gate next_user_instruction=0x50003000000f user_rsp=0x500030002000 bootstrap_rsp0=restored frames_returned=11 if=masked timer_choice=off scheduler=off",
    );
    Ok(())
}

#[cfg(feature = "m5o-ci-self-test")]
pub fn test_syscall_sysret_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld, image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64, architecture: Architecture::X86_64,
    };
    let bootstrap = crate::arch::current_tss_rsp0();
    let before = crate::memory::frame_reuse_stats()?;
    let live_before = process::live_count();
    let mut process = ProcessObject::new_with_private_kernel_stack(profile)?;
    process.map_one_user_leaf(0x0000_5000_3000_0000, PagePermissions::user_read_execute())?;
    process.map_adjacent_user_stack()?;
    let handle = handle::insert(Arc::new(FwObject::Process(process)), Rights::READ)?;
    let strong = handle::get(handle, Rights::READ)?;
    let proof = match strong.as_ref() {
        FwObject::Process(p) => p.ci_syscall_sysret()?,
        _ => return Err(ObjectError::WrongObjectType),
    };
    if proof.original_kernel_cr3 != proof.restored_kernel_cr3
        || proof.original_kernel_cr3 == proof.observed_process_cr3
        || crate::arch::current_tss_rsp0() != bootstrap
        || process::live_count() != live_before + 1 {
        return Err(ObjectError::SelfTestFailed);
    }
    drop(strong);
    close(handle)?;
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 10
        || process::live_count() != live_before {
        return Err(ObjectError::SelfTestFailed);
    }
    crate::arch::serial::println(
        "FreeWorldOS: M5-O SYSCALL/SYSRET self-test: passed msrs=STAR,LSTAR,SFMASK syscall=real sysret=real continuation=0x50003000000c final_test_trap=0x50003000000e user_rsp=0x500030002000 bootstrap_rsp0=restored frames_returned=10 if=masked scheduler=off timer_choice=off",
    );
    Ok(())
}

#[cfg(feature = "m5aa-ci-self-test")]
pub fn test_task_syscall_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld, image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64, architecture: Architecture::X86_64,
    };
    let before = crate::memory::frame_reuse_stats()?;
    let live = process::live_count();
    let bootstrap = crate::arch::current_tss_rsp0();
    let mut process = ProcessObject::new_with_syscall_task(profile)?;
    process.map_one_user_leaf(0x0000_5000_3000_0000, PagePermissions::user_read_execute())?;
    process.map_adjacent_user_stack()?;
    let proof = process.ci_task_syscall()?;
    if proof.original_kernel_cr3 != proof.restored_kernel_cr3
        || proof.original_kernel_cr3 == proof.observed_process_cr3
        || crate::arch::current_tss_rsp0() != bootstrap
        || process::live_count() != live + 1 {
        return Err(ObjectError::SelfTestFailed);
    }
    drop(process);
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 10
        || process::live_count() != live {
        return Err(ObjectError::SelfTestFailed);
    }
    crate::arch::serial::println(
        "FreeWorldOS: M5-AA complete: task_context=owned private_kernel_stack=yes bootstrap_rsp0=distinct sysret=real frames_returned=10 no_scheduler=yes",
    );
    Ok(())
}

#[cfg(feature = "m5ab-ci-self-test")]
pub fn test_task_native_dispatch_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld, image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64, architecture: Architecture::X86_64,
    };
    let before = crate::memory::frame_reuse_stats()?;
    let live = process::live_count();
    let bootstrap = crate::arch::current_tss_rsp0();
    let mut process = ProcessObject::new_with_syscall_task(profile)?;
    process.map_one_user_leaf(0x0000_5000_3000_0000, PagePermissions::user_read_execute())?;
    process.map_adjacent_user_stack()?;
    let proof = process.ci_task_native_dispatch()?;
    if proof.original_kernel_cr3 != proof.restored_kernel_cr3
        || proof.original_kernel_cr3 == proof.observed_process_cr3
        || crate::arch::current_tss_rsp0() != bootstrap
        || process::live_count() != live + 1 { return Err(ObjectError::SelfTestFailed); }
    drop(process);
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 10
        || process::live_count() != live { return Err(ObjectError::SelfTestFailed); }
    crate::arch::serial::println(
        "FreeWorldOS: M5-AB complete: task_owned_dispatch=yes syscalls=4 signed64_errors=yes user_copy=page_walk frames_returned=10 scheduler=off timer_choice=off",
    );
    Ok(())
}

#[cfg(feature = "m5p-ci-self-test")]
pub fn test_one_number_syscall_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld, image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64, architecture: Architecture::X86_64,
    };
    let bootstrap = crate::arch::current_tss_rsp0();
    let before = crate::memory::frame_reuse_stats()?;
    let live_before = process::live_count();
    let mut process = ProcessObject::new_with_private_kernel_stack(profile)?;
    process.map_one_user_leaf(0x0000_5000_3000_0000, PagePermissions::user_read_execute())?;
    process.map_adjacent_user_stack()?;
    let handle = handle::insert(Arc::new(FwObject::Process(process)), Rights::READ)?;
    let strong = handle::get(handle, Rights::READ)?;
    let proof = match strong.as_ref() {
        FwObject::Process(p) => p.ci_one_number_syscall()?,
        _ => return Err(ObjectError::WrongObjectType),
    };
    if proof.original_kernel_cr3 != proof.restored_kernel_cr3
        || proof.original_kernel_cr3 == proof.observed_process_cr3
        || crate::arch::current_tss_rsp0() != bootstrap
        || process::live_count() != live_before + 1 {
        return Err(ObjectError::SelfTestFailed);
    }
    drop(strong);
    close(handle)?;
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 10
        || process::live_count() != live_before {
        return Err(ObjectError::SelfTestFailed);
    }
    crate::arch::serial::println(
        "FreeWorldOS: M5-P numbered syscall: passed number=1 arg=41 result=42 user_checked=cmp syscall=real sysret=real continuation=0x500030000010 final_test_trap=0x500030000018 user_rsp=0x500030002000 msrs=restored bootstrap_rsp0=restored frames_returned=10 if=masked scheduler=off timer_choice=off",
    );
    Ok(())
}

#[cfg(feature = "m5q-ci-self-test")]
pub fn test_freeworld_elf_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld, image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64, architecture: Architecture::X86_64,
    };
    crate::exec::freeworld_elf::ci_parser_test()
        .map_err(|_| ObjectError::SelfTestFailed)?;
    let valid = crate::exec::freeworld_elf::test_image(1);
    let bad = crate::exec::freeworld_elf::test_image(2);
    if crate::exec::freeworld_elf::validate(&bad).err()
        != Some(crate::exec::freeworld_elf::Error::BadAbiVersion)
    { return Err(ObjectError::SelfTestFailed); }
    let bootstrap = crate::arch::current_tss_rsp0();
    let before = crate::memory::frame_reuse_stats()?;
    let live_before = process::live_count();
    let mut process = ProcessObject::new_with_private_kernel_stack(profile)?;
    process.map_one_user_leaf(0x0000_5000_3000_0000, PagePermissions::user_read_execute())?;
    process.map_adjacent_user_stack()?;
    let handle = handle::insert(Arc::new(FwObject::Process(process)), Rights::READ)?;
    let strong = handle::get(handle, Rights::READ)?;
    // Negative ABI case must go through the same process loader interface,
    // and must be rejected before staging code or entering Ring 3.
    match strong.as_ref() {
        FwObject::Process(p) => {
            if p.ci_load_freeworld_elf(&bad).is_ok() {
                return Err(ObjectError::SelfTestFailed);
            }
        },
        _ => return Err(ObjectError::WrongObjectType),
    }
    let proof = match strong.as_ref() {
        FwObject::Process(p) => p.ci_load_freeworld_elf(&valid)?,
        _ => return Err(ObjectError::WrongObjectType),
    };
    if proof.original_kernel_cr3 != proof.restored_kernel_cr3
        || proof.original_kernel_cr3 == proof.observed_process_cr3
        || crate::arch::current_tss_rsp0() != bootstrap
        || process::live_count() != live_before + 1 {
        return Err(ObjectError::SelfTestFailed);
    }
    drop(strong);
    close(handle)?;
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 10
        || process::live_count() != live_before {
        return Err(ObjectError::SelfTestFailed);
    }
    crate::arch::serial::println(
        "FreeWorldOS: M5-Q FreeWorld ELF loader: passed note=FreeWorldOS type=1 abi=1 bad_version=rejected pt_load=rx entry=0x500030000000 syscall=1 arg=41 result=42 user_checked=cmp msrs=restored bootstrap_rsp0=restored frames_returned=10 if=masked scheduler=off timer_choice=off",
    );
    Ok(())
}

#[cfg(feature = "m5r-ci-self-test")]
pub fn test_hello_from_freeworld_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld, image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64, architecture: Architecture::X86_64,
    };
    crate::exec::freeworld_elf::ci_parser_test()
        .map_err(|_| ObjectError::SelfTestFailed)?;
    let valid = crate::exec::freeworld_elf::hello_image();
    let bad = crate::exec::freeworld_elf::test_image(2);
    if crate::exec::freeworld_elf::validate(&bad).err()
        != Some(crate::exec::freeworld_elf::Error::BadAbiVersion)
    { return Err(ObjectError::SelfTestFailed); }
    let bootstrap = crate::arch::current_tss_rsp0();
    let before = crate::memory::frame_reuse_stats()?;
    let live_before = process::live_count();
    let mut process = ProcessObject::new_with_private_kernel_stack(profile)?;
    process.map_one_user_leaf(0x0000_5000_3000_0000, PagePermissions::user_read_execute())?;
    process.map_adjacent_user_stack()?;
    let handle = handle::insert(Arc::new(FwObject::Process(process)), Rights::READ)?;
    let strong = handle::get(handle, Rights::READ)?;
    // Negative ABI case must go through the same process loader interface,
    // and must be rejected before staging code or entering Ring 3.
    match strong.as_ref() {
        FwObject::Process(p) => {
            if p.ci_hello_elf(&bad).is_ok() {
                return Err(ObjectError::SelfTestFailed);
            }
        },
        _ => return Err(ObjectError::WrongObjectType),
    }
    let proof = match strong.as_ref() {
        FwObject::Process(p) => p.ci_hello_elf(&valid)?,
        _ => return Err(ObjectError::WrongObjectType),
    };
    if proof.original_kernel_cr3 != proof.restored_kernel_cr3
        || proof.original_kernel_cr3 == proof.observed_process_cr3
        || crate::arch::current_tss_rsp0() != bootstrap
        || process::live_count() != live_before + 1 {
        return Err(ObjectError::SelfTestFailed);
    }
    drop(strong);
    close(handle)?;
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 10
        || process::live_count() != live_before {
        return Err(ObjectError::SelfTestFailed);
    }
    crate::arch::serial::println(
        "FreeWorldOS: M5-R sentence syscall: passed note=FreeWorldOS abi=1 syscall=2 pointer=0x500030000080 length=20 user_checked=20 user_rsp=unchanged msrs=restored bootstrap_rsp0=restored frames_returned=10 if=masked scheduler=off timer_choice=off",
    );
    Ok(())
}

#[cfg(feature = "m5s-ci-self-test")]
pub fn test_unmapped_then_hello_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld, image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64, architecture: Architecture::X86_64,
    };
    crate::exec::freeworld_elf::ci_parser_test()
        .map_err(|_| ObjectError::SelfTestFailed)?;
    let valid = crate::exec::freeworld_elf::invalid_then_hello_image();
    let bad = crate::exec::freeworld_elf::test_image(2);
    if crate::exec::freeworld_elf::validate(&bad).err()
        != Some(crate::exec::freeworld_elf::Error::BadAbiVersion)
    { return Err(ObjectError::SelfTestFailed); }
    let bootstrap = crate::arch::current_tss_rsp0();
    let before = crate::memory::frame_reuse_stats()?;
    let live_before = process::live_count();
    let mut process = ProcessObject::new_with_private_kernel_stack(profile)?;
    process.map_one_user_leaf(0x0000_5000_3000_0000, PagePermissions::user_read_execute())?;
    process.map_adjacent_user_stack()?;
    let handle = handle::insert(Arc::new(FwObject::Process(process)), Rights::READ)?;
    let strong = handle::get(handle, Rights::READ)?;
    // Negative ABI case must go through the same process loader interface,
    // and must be rejected before staging code or entering Ring 3.
    match strong.as_ref() {
        FwObject::Process(p) => {
            if p.ci_unmapped_then_hello(&bad).is_ok() {
                return Err(ObjectError::SelfTestFailed);
            }
        },
        _ => return Err(ObjectError::WrongObjectType),
    }
    let proof = match strong.as_ref() {
        FwObject::Process(p) => p.ci_unmapped_then_hello(&valid)?,
        _ => return Err(ObjectError::WrongObjectType),
    };
    if proof.original_kernel_cr3 != proof.restored_kernel_cr3
        || proof.original_kernel_cr3 == proof.observed_process_cr3
        || crate::arch::current_tss_rsp0() != bootstrap
        || process::live_count() != live_before + 1 {
        return Err(ObjectError::SelfTestFailed);
    }
    drop(strong);
    close(handle)?;
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 10
        || process::live_count() != live_before {
        return Err(ObjectError::SelfTestFailed);
    }
    crate::arch::serial::println(
        "FreeWorldOS: M5-S unmapped pointer: passed bad_ptr=0x500030003000 error=-8193 without_fault=true good_ptr=0x500030000080 good_result=20 frames_returned=10 msrs=restored bootstrap_rsp0=restored scheduler=off timer_choice=off",
    );
    Ok(())
}

#[cfg(feature = "m5u-ci-self-test")]
pub fn test_cross_page_copy_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    const CODE: u64 = 0x0000_5000_3000_0000;
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld, image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64, architecture: Architecture::X86_64,
    };
    let before = crate::memory::frame_reuse_stats()?;
    let live_before = process::live_count();
    let bootstrap = crate::arch::current_tss_rsp0();
    let mut process = ProcessObject::new_with_private_kernel_stack(profile)?;
    process.map_one_user_leaf(CODE, PagePermissions::user_read_execute())?;
    let bad_second = crate::exec::freeworld_elf::cross_page_image(CODE + 4096 - 8, fw_abi::FW_ERR_USER_ADDRESS);
    let first = process.ci_cross_page(&bad_second, false)?;
    if first.original_kernel_cr3 != first.restored_kernel_cr3
        || crate::arch::memory::ci_user_copy_count() != 0
    { return Err(ObjectError::SelfTestFailed); }
    crate::arch::serial::println("FreeWorldOS: M5-U second-unmapped: result=-8193 copied=0");
    // Same process: install only the adjacent writable page, no scheduling.
    process.map_adjacent_user_stack()?;
    let good = crate::exec::freeworld_elf::cross_page_image(CODE + 4096 - 8, 20);
    let success = process.ci_cross_page(&good, true)?;
    if success.original_kernel_cr3 != success.restored_kernel_cr3
        || crate::arch::memory::ci_user_copy_count() != 20
    { return Err(ObjectError::SelfTestFailed); }
    crate::arch::serial::println("FreeWorldOS: M5-U two-page success: segments=8+12 copied=20");
    for bad_pointer in [CODE + 0x3000, u64::MAX - 7] {
        let bad = crate::exec::freeworld_elf::cross_page_image(bad_pointer, fw_abi::FW_ERR_USER_ADDRESS);
        let proof = process.ci_cross_page(&bad, false)?;
        if proof.original_kernel_cr3 != proof.restored_kernel_cr3
            || crate::arch::memory::ci_user_copy_count() != 0
        { return Err(ObjectError::SelfTestFailed); }
    }
    drop(process);
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 10
        || process::live_count() != live_before || crate::arch::current_tss_rsp0() != bootstrap
    { return Err(ObjectError::SelfTestFailed); }
    crate::arch::serial::println("FreeWorldOS: M5-U complete: second_unmapped=-8193 first_unmapped=-8193 overflow=-8193 zero_copy_on_reject=yes frames_returned=10 scheduler=off timer_choice=off concurrent_unmap=unsupported");
    Ok(())
}

#[cfg(feature = "m5u-ci-self-test")]
pub fn test_three_page_user_copy_ci_self_test() -> Result<(), ObjectError> {
    use crate::{exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat}, memory::PagePermissions};
    let before = crate::memory::frame_reuse_stats()?;
    let live_before = process::live_count();
    let profile = ExecutionProfile { environment: Environment::FreeWorld,
        image_format: ImageFormat::FreeWorld, abi: Abi::FreeWorld64,
        architecture: Architecture::X86_64 };
    let mut process = ProcessObject::new(profile)?;
    process.map_one_user_leaf(0x0000_5000_3000_0000, PagePermissions::user_read_execute())?;
    process.map_adjacent_user_stack()?;
    process.map_test_callgate()?;
    process.ci_three_page_user_copy()?;
    drop(process);
    let after = crate::memory::frame_reuse_stats()?;
    if process::live_count() != live_before
        || after.returned_total.saturating_sub(before.returned_total) != 7
    { return Err(ObjectError::SelfTestFailed); }
    crate::arch::serial::println("FreeWorldOS: M5-Y complete: three_page=ok missing_fourth=rejected copied_on_reject=0 frames_returned=7 scheduler=off timer_choice=off");
    Ok(())
}

#[cfg(feature = "m5z-ci-self-test")]
pub fn test_copy_to_user_ci_self_test() -> Result<(), ObjectError> {
    use crate::{exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat}, memory::PagePermissions};
    let profile = ExecutionProfile { environment: Environment::FreeWorld,
        image_format: ImageFormat::FreeWorld, abi: Abi::FreeWorld64,
        architecture: Architecture::X86_64 };
    let before = crate::memory::frame_reuse_stats()?;
    let live = process::live_count();
    let mut process = ProcessObject::new(profile)?;
    process.map_one_user_leaf(0x0000_5000_3000_0000, PagePermissions::user_read_execute())?;
    process.map_adjacent_user_stack()?;
    process.map_test_callgate()?;
    process.ci_copy_to_user()?;
    drop(process);
    let after = crate::memory::frame_reuse_stats()?;
    if process::live_count() != live
        || after.returned_total.saturating_sub(before.returned_total) != 7 {
        return Err(ObjectError::SelfTestFailed);
    }
    crate::arch::serial::println("FreeWorldOS: M5-Z complete: writable=ok readonly=rejected missing=rejected zero_copy_on_reject=yes frames_returned=7 scheduler=off timer_choice=off");
    Ok(())
}

#[cfg(feature = "m5t-ci-self-test")]
pub fn test_page_walk_then_hello_ci_self_test() -> Result<(), ObjectError> {
    use crate::{
        exec::profile::{Abi, Architecture, Environment, ExecutionProfile, ImageFormat},
        memory::PagePermissions,
    };
    let profile = ExecutionProfile {
        environment: Environment::FreeWorld, image_format: ImageFormat::FreeWorld,
        abi: Abi::FreeWorld64, architecture: Architecture::X86_64,
    };
    crate::exec::freeworld_elf::ci_parser_test()
        .map_err(|_| ObjectError::SelfTestFailed)?;
    let valid = crate::exec::freeworld_elf::invalid_then_hello_image();
    let bad = crate::exec::freeworld_elf::test_image(2);
    if crate::exec::freeworld_elf::validate(&bad).err()
        != Some(crate::exec::freeworld_elf::Error::BadAbiVersion)
    { return Err(ObjectError::SelfTestFailed); }
    let bootstrap = crate::arch::current_tss_rsp0();
    let before = crate::memory::frame_reuse_stats()?;
    let live_before = process::live_count();
    let mut process = ProcessObject::new_with_private_kernel_stack(profile)?;
    process.map_one_user_leaf(0x0000_5000_3000_0000, PagePermissions::user_read_execute())?;
    process.map_adjacent_user_stack()?;
    let handle = handle::insert(Arc::new(FwObject::Process(process)), Rights::READ)?;
    let strong = handle::get(handle, Rights::READ)?;
    // Negative ABI case must go through the same process loader interface,
    // and must be rejected before staging code or entering Ring 3.
    match strong.as_ref() {
        FwObject::Process(p) => {
            if p.ci_page_walk_then_hello(&bad).is_ok() {
                return Err(ObjectError::SelfTestFailed);
            }
        },
        _ => return Err(ObjectError::WrongObjectType),
    }
    let proof = match strong.as_ref() {
        FwObject::Process(p) => p.ci_page_walk_then_hello(&valid)?,
        _ => return Err(ObjectError::WrongObjectType),
    };
    if proof.original_kernel_cr3 != proof.restored_kernel_cr3
        || proof.original_kernel_cr3 == proof.observed_process_cr3
        || crate::arch::current_tss_rsp0() != bootstrap
        || process::live_count() != live_before + 1 {
        return Err(ObjectError::SelfTestFailed);
    }
    drop(strong);
    close(handle)?;
    let after = crate::memory::frame_reuse_stats()?;
    if after.returned_total.saturating_sub(before.returned_total) != 10
        || process::live_count() != live_before {
        return Err(ObjectError::SelfTestFailed);
    }
    crate::arch::serial::println(
        "FreeWorldOS: M5-T page-table walk: passed bad_ptr=0x500030003000 error=-8193 without_fault=true good_ptr=0x500030000080 good_result=20 frames_returned=10 msrs=restored bootstrap_rsp0=restored scheduler=off timer_choice=off",
    );
    Ok(())
}

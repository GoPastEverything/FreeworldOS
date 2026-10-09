use alloc::sync::Arc;

pub mod counter;
pub mod handle;
pub mod module;
pub mod process;
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

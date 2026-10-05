use core::{
    mem::size_of,
    sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};

use crate::memory::{self, PagePermissions, PhysFrame, PAGE_SIZE};

use super::ObjectError;

pub const TASK_STACK_PAGES: usize = 4;
const TASK_STACK_REGION_START: u64 = 0xffff_a000_0000_0000;
const TASK_STACK_SLOT_PAGES: u64 = TASK_STACK_PAGES as u64 + 1;
const TASK_STACK_SLOT_BYTES: u64 = TASK_STACK_SLOT_PAGES * PAGE_SIZE;
const MAX_TASK_STACK_SLOTS: u64 = 1024;

static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_STACK_SLOT: AtomicU64 = AtomicU64::new(0);

#[repr(C)]
#[derive(Clone, Copy)]
struct SavedRegisterFrame {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    rbp: u64,
    rbx: u64,
    rip: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TaskState {
    Created = 0,
    Runnable = 1,
    Running = 2,
    Stopped = 3,
}

impl TaskState {
    fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Created,
            1 => Self::Runnable,
            2 => Self::Running,
            3 => Self::Stopped,
            _ => panic!("FreeWorld task state corrupt: {value}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskInfo {
    pub id: u64,
    pub state: TaskState,
    pub guard_page: u64,
    pub stack_bottom: u64,
    pub stack_top: u64,
    pub initial_stack_pointer: u64,
    pub stack_pages: usize,
}

pub struct TaskObject {
    id: u64,
    state: AtomicU8,
    saved_stack_pointer_present: AtomicBool,
    saved_stack_pointer: AtomicU64,
    stack: KernelStack,
}

impl TaskObject {
    pub fn new() -> Result<Self, ObjectError> {
        let id = NEXT_TASK_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| ObjectError::TaskIdExhausted)?;

        Ok(Self {
            id,
            state: AtomicU8::new(TaskState::Created as u8),
            saved_stack_pointer_present: AtomicBool::new(false),
            saved_stack_pointer: AtomicU64::new(0),
            stack: KernelStack::new()?,
        })
    }

    pub fn info(&self) -> TaskInfo {
        TaskInfo {
            id: self.id,
            state: self.state(),
            guard_page: self.stack.guard_page,
            stack_bottom: self.stack.stack_bottom,
            stack_top: self.stack.stack_top,
            initial_stack_pointer: self.stack.initial_stack_pointer(),
            stack_pages: TASK_STACK_PAGES,
        }
    }

    pub(super) fn can_release_stack(&self) -> bool {
        matches!(self.state(), TaskState::Created | TaskState::Stopped)
            && !self.saved_stack_pointer_present.load(Ordering::Acquire)
    }

    pub(crate) fn state(&self) -> TaskState {
        TaskState::from_raw(self.state.load(Ordering::Acquire))
    }

    pub(crate) fn prepare_initial_context(
        &self,
        entry: extern "C" fn() -> !,
    ) -> Result<(), ObjectError> {
        if self.state() != TaskState::Created
            || self.saved_stack_pointer_present.load(Ordering::Acquire)
        {
            return Err(ObjectError::InvalidTaskState);
        }

        let frame_bytes = size_of::<SavedRegisterFrame>() as u64;
        assert_eq!(
            frame_bytes,
            7 * size_of::<u64>() as u64,
            "FreeWorld saved register frame layout changed"
        );

        let entry_rsp = self.stack.initial_stack_pointer();
        let frame_rsp = entry_rsp
            .checked_sub(frame_bytes)
            .ok_or(ObjectError::InvalidTaskState)?;

        assert!(
            frame_rsp >= self.stack.stack_bottom,
            "FreeWorld initial saved register frame escaped task stack"
        );

        let frame = SavedRegisterFrame {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rbp: 0,
            rbx: 0,
            rip: entry as usize as u64,
        };

        // SAFETY: frame_rsp points into this task's exclusively owned RW/NX
        // stack and is naturally aligned for the fixed register frame.
        unsafe {
            core::ptr::write_volatile(frame_rsp as *mut SavedRegisterFrame, frame);
            core::ptr::write_volatile(entry_rsp as *mut u64, 0);
        }

        self.saved_stack_pointer.store(frame_rsp, Ordering::Release);
        self.saved_stack_pointer_present.store(true, Ordering::Release);
        self.state.store(TaskState::Runnable as u8, Ordering::Release);
        Ok(())
    }

    pub(crate) fn begin_running_from_saved(&self) -> Result<(), ObjectError> {
        if self.state() != TaskState::Runnable
            || !self.saved_stack_pointer_present.load(Ordering::Acquire)
            || self.saved_stack_pointer.load(Ordering::Acquire) == 0
        {
            return Err(ObjectError::InvalidTaskState);
        }

        self.saved_stack_pointer_present.store(false, Ordering::Release);
        self.state.store(TaskState::Running as u8, Ordering::Release);
        Ok(())
    }

    pub(crate) fn prepare_running_context_save(&self) -> Result<(), ObjectError> {
        if self.state() != TaskState::Running {
            return Err(ObjectError::InvalidTaskState);
        }

        // The scheduler owns a strong reference before this flag is set. The
        // assembly switch writes the exact saved RSP immediately afterward.
        self.saved_stack_pointer_present.store(true, Ordering::Release);
        self.state.store(TaskState::Runnable as u8, Ordering::Release);
        Ok(())
    }

    pub(crate) fn saved_stack_pointer(&self) -> u64 {
        self.saved_stack_pointer.load(Ordering::Acquire)
    }

    pub(crate) fn saved_stack_pointer_storage(&self) -> *mut u64 {
        self.saved_stack_pointer.as_ptr()
    }

    pub(crate) fn saved_stack_pointer_present(&self) -> bool {
        self.saved_stack_pointer_present.load(Ordering::Acquire)
    }

    pub(crate) fn saved_stack_pointer_in_stack(&self) -> bool {
        let rsp = self.saved_stack_pointer();
        let frame_bytes = size_of::<SavedRegisterFrame>() as u64;
        rsp >= self.stack.stack_bottom
            && rsp
                .checked_add(frame_bytes)
                .is_some_and(|end| end <= self.stack.initial_stack_pointer())
    }

    #[cfg(feature = "m35c2b-ci-self-test")]
    pub(super) fn set_state_for_ci(&self, state: TaskState) {
        self.state.store(state as u8, Ordering::Release);
    }

    #[cfg(feature = "m35c2b-ci-self-test")]
    pub(super) fn set_saved_stack_pointer_present_for_ci(&self, present: bool) {
        self.saved_stack_pointer_present.store(present, Ordering::Release);
    }

    #[cfg(feature = "m35c-ci-self-test")]
    pub fn stack_writable_ci_test(&self) -> bool {
        const BOTTOM_PATTERN: u64 = 0x4657_5441_534b_424f;
        const TOP_PATTERN: u64 = 0x4657_5441_534b_544f;

        // SAFETY: These addresses are inside this task object's mapped,
        // exclusively owned RW/NX stack pages. Volatile access avoids creating
        // long-lived references to memory that will later be unmapped on Drop.
        unsafe {
            let bottom = self.stack.stack_bottom as *mut u64;
            let top = (self.stack.stack_top - 8) as *mut u64;

            core::ptr::write_volatile(bottom, BOTTOM_PATTERN);
            core::ptr::write_volatile(top, TOP_PATTERN);

            core::ptr::read_volatile(bottom) == BOTTOM_PATTERN
                && core::ptr::read_volatile(top) == TOP_PATTERN
        }
    }
}

impl Drop for TaskObject {
    fn drop(&mut self) {
        assert!(
            self.can_release_stack(),
            "FreeWorld attempted to destroy a runnable/running task or a task with saved stack state"
        );
    }
}

struct KernelStack {
    guard_page: u64,
    stack_bottom: u64,
    stack_top: u64,
    guard_reserved: bool,
    frames: [Option<PhysFrame>; TASK_STACK_PAGES],
}

impl KernelStack {
    fn new() -> Result<Self, ObjectError> {
        let slot = NEXT_STACK_SLOT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                (current < MAX_TASK_STACK_SLOTS).then_some(current + 1)
            })
            .map_err(|_| ObjectError::TaskStackSlotsExhausted)?;

        let guard_page = TASK_STACK_REGION_START
            .checked_add(
                slot.checked_mul(TASK_STACK_SLOT_BYTES)
                    .ok_or(ObjectError::TaskStackSlotsExhausted)?,
            )
            .ok_or(ObjectError::TaskStackSlotsExhausted)?;
        let stack_bottom = guard_page
            .checked_add(PAGE_SIZE)
            .ok_or(ObjectError::TaskStackSlotsExhausted)?;
        let stack_top = stack_bottom
            .checked_add(TASK_STACK_PAGES as u64 * PAGE_SIZE)
            .ok_or(ObjectError::TaskStackSlotsExhausted)?;

        memory::reserve_virtual_page(guard_page)?;

        let mut stack = Self {
            guard_page,
            stack_bottom,
            stack_top,
            guard_reserved: true,
            frames: [None; TASK_STACK_PAGES],
        };

        for index in 0..TASK_STACK_PAGES {
            let frame = memory::allocate_frame()?;
            let virtual_address = stack_bottom + index as u64 * PAGE_SIZE;

            if let Err(error) = unsafe {
                memory::map_page(
                    virtual_address,
                    frame,
                    PagePermissions::read_write(),
                )
            } {
                // The mapping never became owner-visible, so this frame can be
                // returned immediately. Previously mapped pages are reclaimed
                // by KernelStack::drop while unwinding this constructor.
                unsafe {
                    memory::free_frame(frame)
                        .expect("FreeWorld task stack failed to return unmapped frame");
                }
                return Err(error.into());
            }

            stack.frames[index] = Some(frame);
        }

        // The guard page itself is intentionally never mapped. C2b also owns
        // its virtual address through the memory reservation table, so generic
        // map_page() callers cannot consume it while this task owns the slot.
        let initial_stack_pointer = stack.initial_stack_pointer();
        assert_eq!(
            initial_stack_pointer & 0xf,
            8,
            "FreeWorld task initial RSP violates System V entry alignment"
        );

        // SAFETY: This is the final 8-byte slot in the mapped stack. It models
        // the return-address-sized slot present at normal System V function
        // entry. The future task trampoline must not return through zero.
        unsafe {
            core::ptr::write_volatile(initial_stack_pointer as *mut u64, 0);
        }

        Ok(stack)
    }

    fn initial_stack_pointer(&self) -> u64 {
        self.stack_top - 8
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        for index in (0..TASK_STACK_PAGES).rev() {
            let Some(expected_frame) = self.frames[index].take() else {
                continue;
            };
            let virtual_address = self.stack_bottom + index as u64 * PAGE_SIZE;
            let unmapped = memory::unmap_page(virtual_address)
                .expect("FreeWorld task stack page disappeared before object drop");

            assert_eq!(
                unmapped,
                expected_frame,
                "FreeWorld task stack mapping changed physical ownership"
            );

            // SAFETY: unmap_page removed the task's owner-visible mapping and
            // the TaskObject owns this frame exclusively.
            unsafe {
                memory::free_frame(unmapped)
                    .expect("FreeWorld task stack failed to return frame");
            }
        }

        if self.guard_reserved {
            memory::release_virtual_page_reservation(self.guard_page)
                .expect("FreeWorld task guard reservation disappeared before object drop");
            self.guard_reserved = false;
        }
    }
}

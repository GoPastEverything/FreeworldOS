use core::sync::atomic::{AtomicU64, Ordering};

use crate::memory::{self, PagePermissions, PhysFrame, PAGE_SIZE};

use super::ObjectError;

pub const TASK_STACK_PAGES: usize = 4;
const TASK_STACK_REGION_START: u64 = 0xffff_a000_0000_0000;
const TASK_STACK_SLOT_PAGES: u64 = TASK_STACK_PAGES as u64 + 1;
const TASK_STACK_SLOT_BYTES: u64 = TASK_STACK_SLOT_PAGES * PAGE_SIZE;
const MAX_TASK_STACK_SLOTS: u64 = 1024;

static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_STACK_SLOT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskState {
    Created,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskInfo {
    pub id: u64,
    pub state: TaskState,
    pub guard_page: u64,
    pub stack_bottom: u64,
    pub stack_top: u64,
    pub stack_pages: usize,
}

pub struct TaskObject {
    id: u64,
    state: TaskState,
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
            state: TaskState::Created,
            stack: KernelStack::new()?,
        })
    }

    pub fn info(&self) -> TaskInfo {
        TaskInfo {
            id: self.id,
            state: self.state,
            guard_page: self.stack.guard_page,
            stack_bottom: self.stack.stack_bottom,
            stack_top: self.stack.stack_top,
            stack_pages: TASK_STACK_PAGES,
        }
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

struct KernelStack {
    guard_page: u64,
    stack_bottom: u64,
    stack_top: u64,
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

        let mut stack = Self {
            guard_page,
            stack_bottom,
            stack_top,
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

        // The guard page itself is intentionally never mapped. The surrounding
        // page-table hierarchy may exist because the stack pages above it are
        // mapped, but its leaf entry remains not-present.
        Ok(stack)
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
    }
}

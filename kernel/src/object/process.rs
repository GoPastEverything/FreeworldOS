use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::{
    exec::profile::ExecutionProfile,
    memory::{MemoryError, PagePermissions, PhysFrame, ProcessAddressSpace},
};

static NEXT_PROCESS_ID: AtomicU64 = AtomicU64::new(1);
static LIVE_PROCESSES: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessIdentity {
    pub object_id: u128,
    pub profile: ExecutionProfile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessInfo {
    pub identity: ProcessIdentity,
    pub address_space_root: PhysFrame,
}

pub struct ProcessObject {
    identity: ProcessIdentity,
    address_space: ProcessAddressSpace,
    #[cfg(feature = "m5i-ci-self-test")]
    kernel_stack: Option<super::task::KernelStack>,
    #[cfg(feature = "m5aa-ci-self-test")]
    user_task: Option<alloc::boxed::Box<super::task::UserSyscallTask>>,
}

impl ProcessObject {
    pub fn new(profile: ExecutionProfile) -> Result<Self, MemoryError> {
        let id = NEXT_PROCESS_ID.fetch_add(1, Ordering::AcqRel);
        assert_ne!(id, 0, "FreeWorld process id space exhausted");

        let address_space = ProcessAddressSpace::new()?;
        LIVE_PROCESSES.fetch_add(1, Ordering::AcqRel);

        Ok(Self {
            identity: ProcessIdentity {
                object_id: u128::from(id),
                profile,
            },
            address_space,
            #[cfg(feature = "m5i-ci-self-test")]
            kernel_stack: None,
            #[cfg(feature = "m5aa-ci-self-test")]
            user_task: None,
        })
    }

    pub fn map_one_user_leaf(
        &mut self,
        address: u64,
        permissions: PagePermissions,
    ) -> Result<(), MemoryError> {
        self.address_space.map_one_user_leaf(address, permissions)
    }

    #[cfg(feature = "m5h-ci-self-test")]
    pub fn map_adjacent_user_stack(&mut self) -> Result<(), MemoryError> {
        self.address_space.map_adjacent_user_stack()
    }

    #[cfg(feature = "m5h-ci-self-test")]
    pub fn ci_stack_push(&self) -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError> {
        self.address_space.ci_stack_push()
    }

    #[cfg(feature = "m5i-ci-self-test")]
    pub fn new_with_private_kernel_stack(profile: ExecutionProfile) -> Result<Self, super::ObjectError> {
        // Stack must be mapped in the higher half BEFORE the process PML4
        // snapshots its kernel-half entries.
        let stack = super::task::KernelStack::new()?;
        let mut process = Self::new(profile)?;
        process.kernel_stack = Some(stack);
        Ok(process)
    }

    #[cfg(feature = "m5aa-ci-self-test")]
    pub fn new_with_syscall_task(profile: ExecutionProfile) -> Result<Self, super::ObjectError> {
        // Allocate the guarded task stack BEFORE cloning higher-half page
        // table mappings into this process's private CR3.
        // Keep ProcessObject / FwObject thin: embedding a 4-page stack
        // descriptor inline would increase the size of *every* Arc<FwObject>
        // and exhaust the fixed small-object heap class in existing tests.
        let task = alloc::boxed::Box::new(super::task::UserSyscallTask::new()?);
        let mut process = Self::new(profile)?;
        process.user_task = Some(task);
        Ok(process)
    }

    #[cfg(feature = "m5aa-ci-self-test")]
    pub fn ci_task_syscall(&self)
        -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError>
    {
        let task = self.user_task.as_ref().ok_or(MemoryError::PageNotMapped)?;
        self.address_space.ci_task_syscall(task.context())
    }

    #[cfg(feature = "m5ab-ci-self-test")]
    pub fn ci_task_native_dispatch(&self)
        -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError>
    {
        let task = self.user_task.as_ref().ok_or(MemoryError::PageNotMapped)?;
        self.address_space.ci_task_native_dispatch(task.context())
    }

    /// Start the admitted native user task (IF=0, no scheduler). It can be
    /// started once only, and reaches Stopped through FW_SYS_EXIT.
    #[cfg(feature = "m5ac-ci-self-test")]
    pub fn start_native(&mut self) -> Result<u64, super::ObjectError> {
        let task = self.user_task.as_mut().ok_or(super::ObjectError::InvalidTaskState)?;
        task.mark_running()?;
        match self.address_space.start_native_process(task.context()) {
            Ok(proof) => {
                task.mark_stopped(proof.exit_status);
                Ok(proof.exit_status)
            }
            Err(e) => {
                task.restore_created();
                Err(e.into())
            }
        }
    }

    #[cfg(feature = "m5ac-ci-self-test")]
    pub fn lifecycle(&self) -> Result<super::task::UserLifecycleState, super::ObjectError> {
        let task = self.user_task.as_ref().ok_or(super::ObjectError::InvalidTaskState)?;
        Ok(task.lifecycle())
    }

    #[cfg(feature = "m5j-ci-self-test")]
    pub fn ci_private_stack_bounds(&self) -> Result<(u64, u64), MemoryError> {
        Ok(self.kernel_stack.as_ref().ok_or(MemoryError::PageNotMapped)?.bounds())
    }

    #[cfg(feature = "m5n-ci-self-test")]
    pub fn map_test_callgate(&mut self) -> Result<(), MemoryError> {
        self.address_space.map_test_callgate()
    }

    #[cfg(feature = "m5z-ci-self-test")]
    pub fn ci_copy_to_user(&self) -> Result<(), MemoryError> {
        self.address_space.ci_copy_to_user()
    }

    #[cfg(feature = "m5u-ci-self-test")]
    pub fn ci_three_page_user_copy(&self) -> Result<(), MemoryError> {
        self.address_space.ci_three_page_user_copy()
    }

    #[cfg(feature = "m5u-ci-self-test")]
    pub fn ci_cross_page(&self, bytes: &[u8], expect_success: bool)
        -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError>
    {
        let (bottom, top) = self.kernel_stack.as_ref().ok_or(MemoryError::PageNotMapped)?.bounds();
        self.address_space.ci_cross_page(bytes, bottom, top, expect_success)
    }

    #[cfg(feature = "m5t-ci-self-test")]
    pub fn ci_page_walk_then_hello(&self, bytes: &[u8])
        -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError>
    {
        let (bottom, top) = self.kernel_stack.as_ref().ok_or(MemoryError::PageNotMapped)?.bounds();
        self.address_space.ci_page_walk_then_hello(bytes, bottom, top)
    }

    #[cfg(feature = "m5s-ci-self-test")]
    pub fn ci_unmapped_then_hello(&self, bytes: &[u8])
        -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError>
    {
        let (bottom, top) = self.kernel_stack.as_ref().ok_or(MemoryError::PageNotMapped)?.bounds();
        self.address_space.ci_unmapped_then_hello(bytes, bottom, top)
    }

    #[cfg(feature = "m5r-ci-self-test")]
    pub fn ci_hello_elf(&self, bytes: &[u8])
        -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError>
    {
        let (bottom, top) = self.kernel_stack.as_ref().ok_or(MemoryError::PageNotMapped)?.bounds();
        self.address_space.ci_hello_elf(bytes, bottom, top)
    }

    #[cfg(feature = "m5q-ci-self-test")]
    pub fn ci_load_freeworld_elf(&self, bytes: &[u8])
        -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError>
    {
        let (bottom, top) = self.kernel_stack.as_ref().ok_or(MemoryError::PageNotMapped)?.bounds();
        self.address_space.ci_load_freeworld_elf(bytes, bottom, top)
    }

    #[cfg(feature = "m5p-ci-self-test")]
    pub fn ci_one_number_syscall(&self) -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError> {
        let (bottom, top) = self.kernel_stack.as_ref().ok_or(MemoryError::PageNotMapped)?.bounds();
        self.address_space.ci_one_number_syscall(bottom, top)
    }

    #[cfg(feature = "m5o-ci-self-test")]
    pub fn ci_syscall_sysret(&self) -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError> {
        let (bottom, top) = self.kernel_stack.as_ref().ok_or(MemoryError::PageNotMapped)?.bounds();
        self.address_space.ci_syscall_sysret(bottom, top)
    }

    #[cfg(feature = "m5n-ci-self-test")]
    pub fn ci_callgate_proof(&self) -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError> {
        let (bottom, top) = self.kernel_stack.as_ref().ok_or(MemoryError::PageNotMapped)?.bounds();
        self.address_space.ci_callgate_proof(bottom, top)
    }

    #[cfg(feature = "m5m-ci-self-test")]
    pub fn ci_stage_explicit_queue(&self, is_a: bool) -> Result<(), MemoryError> {
        self.address_space.ci_stage_explicit_queue(is_a)
    }

    #[cfg(feature = "m5m-ci-self-test")]
    pub fn ci_check_explicit_queue_marker(&self) -> Result<(), MemoryError> {
        self.address_space.ci_check_explicit_queue_marker()
    }

    #[cfg(feature = "m5l-ci-self-test")]
    pub fn ci_private_rsp0_as_b(&self, a_rsp0: u64)
        -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError>
    {
        let stack = self.kernel_stack.as_ref().ok_or(MemoryError::PageNotMapped)?;
        let (bottom, top) = stack.bounds();
        self.address_space.ci_private_rsp0_as_b(bottom, top, a_rsp0)
    }

    #[cfg(feature = "m5l-ci-self-test")]
    pub fn ci_suspend_a_b_a(&self, run_b: impl FnOnce())
        -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError>
    {
        let stack = self.kernel_stack.as_ref().ok_or(MemoryError::PageNotMapped)?;
        let (bottom, top) = stack.bounds();
        self.address_space.ci_suspend_a_b_a(bottom, top, run_b)
    }

    #[cfg(feature = "m5k-ci-self-test")]
    pub fn ci_suspend_resume(&self) -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError> {
        let stack = self.kernel_stack.as_ref().ok_or(MemoryError::PageNotMapped)?;
        let (bottom, top) = stack.bounds();
        self.address_space.ci_suspend_resume(bottom, top)
    }

    #[cfg(feature = "m5i-ci-self-test")]
    pub fn ci_private_rsp0(&self) -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError> {
        let stack = self.kernel_stack.as_ref().ok_or(MemoryError::PageNotMapped)?;
        let (bottom, top) = stack.bounds();
        self.address_space.ci_private_rsp0(bottom, top)
    }

    pub fn inspect_user_leaf(
        &self,
    ) -> Result<crate::arch::memory::InactiveUserLeafInfo, MemoryError> {
        self.address_space.inspect_user_leaf()
    }

    #[cfg(feature = "m5e-ci-self-test")]
    pub fn ci_probe_user_leaf(&self, pattern: u64) -> Result<bool, MemoryError> {
        self.address_space.ci_probe_user_leaf(pattern)
    }

    #[cfg(feature = "m5f-ci-self-test")]
    pub fn ci_cr3_roundtrip(
        &self,
        pattern: u64,
    ) -> Result<crate::arch::memory::ControlledCr3Proof, MemoryError> {
        self.address_space.ci_cr3_roundtrip(pattern)
    }

    #[cfg(feature = "m5g-ci-self-test")]
    pub fn ci_ring3_process_roundtrip(
        &self,
    ) -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError> {
        self.address_space.ci_ring3_process_roundtrip()
    }

    pub fn info(&self) -> ProcessInfo {
        ProcessInfo {
            identity: self.identity,
            address_space_root: self.address_space.root_frame(),
        }
    }

    pub(crate) fn inspect_address_space(
        &self,
    ) -> Result<crate::arch::memory::ProcessAddressSpaceRootInfo, MemoryError> {
        self.address_space.inspect()
    }
}

impl Drop for ProcessObject {
    fn drop(&mut self) {
        let previous = LIVE_PROCESSES.fetch_sub(1, Ordering::AcqRel);
        assert!(previous != 0, "FreeWorld process live-count underflow");
        // ProcessAddressSpace::drop runs immediately after this Drop body and
        // returns the inactive, empty lower-half PML4 root to the recycler.
    }
}

pub fn live_count() -> usize {
    LIVE_PROCESSES.load(Ordering::Acquire)
}

/// Explicitly reclaim an exited process. A Created/Running process cannot
/// be reaped through this lifecycle API. No implicit scheduler cleanup.
#[cfg(feature = "m5ac-ci-self-test")]
pub fn reap_stopped(slot: &mut Option<ProcessObject>) -> Result<u64, super::ObjectError> {
    let status = match slot.as_ref().ok_or(super::ObjectError::InvalidTaskState)?.lifecycle()? {
        super::task::UserLifecycleState::Stopped(code) => code,
        _ => return Err(super::ObjectError::InvalidTaskState),
    };
    drop(slot.take());
    Ok(status)
}

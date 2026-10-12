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

    #[cfg(feature = "m5j-ci-self-test")]
    pub fn ci_private_stack_bounds(&self) -> Result<(u64, u64), MemoryError> {
        Ok(self.kernel_stack.as_ref().ok_or(MemoryError::PageNotMapped)?.bounds())
    }

    #[cfg(feature = "m5n-ci-self-test")]
    pub fn map_test_callgate(&mut self) -> Result<(), MemoryError> {
        self.address_space.map_test_callgate()
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

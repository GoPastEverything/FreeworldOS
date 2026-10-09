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
        })
    }

    pub fn map_one_user_leaf(
        &mut self,
        address: u64,
        permissions: PagePermissions,
    ) -> Result<(), MemoryError> {
        self.address_space.map_one_user_leaf(address, permissions)
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

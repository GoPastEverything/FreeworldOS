use core::sync::atomic::{AtomicU64, Ordering};

pub mod address_space;
pub mod heap;

pub const PAGE_SIZE: u64 = 4096;

const MAX_VIRTUAL_PAGE_RESERVATIONS: usize = 1024;
static RESERVED_VIRTUAL_PAGES: [AtomicU64; MAX_VIRTUAL_PAGE_RESERVATIONS] =
    [const { AtomicU64::new(0) }; MAX_VIRTUAL_PAGE_RESERVATIONS];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysFrame {
    pub start: u64,
}

pub struct ProcessAddressSpace {
    root_frame: PhysFrame,
    user_leaf: Option<crate::arch::memory::InactiveUserLeaf>,
}

impl ProcessAddressSpace {
    pub fn new() -> Result<Self, MemoryError> {
        let root_frame = crate::arch::memory::create_process_address_space_root()?;
        Ok(Self { root_frame, user_leaf: None })
    }

    pub const fn root_frame(&self) -> PhysFrame {
        self.root_frame
    }

    // Bootstrap one-leaf construction runs while the ProcessObject is still
    // uniquely owned, before publication inside an Arc or handle table.
    pub fn map_one_user_leaf(
        &mut self,
        address: u64,
        permissions: PagePermissions,
    ) -> Result<(), MemoryError> {
        if self.user_leaf.is_some() {
            return Err(MemoryError::ProcessUserLeafAlreadyMapped);
        }
        let leaf = crate::arch::memory::map_one_inactive_user_leaf(
            self.root_frame, address, permissions
        )?;
        self.user_leaf = Some(leaf);
        Ok(())
    }

    pub fn inspect(
        &self,
    ) -> Result<crate::arch::memory::ProcessAddressSpaceRootInfo, MemoryError> {
        crate::arch::memory::inspect_process_address_space_root(self.root_frame)
    }

    pub fn inspect_user_leaf(
        &self,
    ) -> Result<crate::arch::memory::InactiveUserLeafInfo, MemoryError> {
        let leaf = self.user_leaf.as_ref().ok_or(MemoryError::PageNotMapped)?;
        crate::arch::memory::inspect_inactive_user_leaf(self.root_frame, leaf)
    }

    #[cfg(feature = "m5e-ci-self-test")]
    pub fn ci_probe_user_leaf(&self, pattern: u64) -> Result<bool, MemoryError> {
        let leaf = self.user_leaf.as_ref().ok_or(MemoryError::PageNotMapped)?;
        crate::arch::memory::ci_probe_inactive_user_leaf(
            self.root_frame, leaf, pattern
        )
    }

    #[cfg(feature = "m5f-ci-self-test")]
    pub fn ci_cr3_roundtrip(
        &self,
        pattern: u64,
    ) -> Result<crate::arch::memory::ControlledCr3Proof, MemoryError> {
        let leaf = self.user_leaf.as_ref().ok_or(MemoryError::PageNotMapped)?;
        crate::arch::memory::ci_controlled_cr3_roundtrip(
            self.root_frame, leaf, pattern
        )
    }

    #[cfg(feature = "m5g-ci-self-test")]
    pub fn ci_ring3_process_roundtrip(
        &self,
    ) -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError> {
        let leaf = self.user_leaf.as_ref().ok_or(MemoryError::PageNotMapped)?;
        leaf.ci_ring3_on_process_root(self.root_frame)
    }
}

impl Drop for ProcessAddressSpace {
    fn drop(&mut self) {
        if let Some(leaf) = self.user_leaf.take() {
            crate::arch::memory::destroy_inactive_user_leaf(self.root_frame, leaf)
                .expect("FreeWorld inactive process user-leaf teardown failed");
        }
        // No process CR3 may be active at this point. The root contains no
        // process-owned lower-half entries after leaf teardown.
        unsafe {
            crate::arch::memory::destroy_process_address_space_root(self.root_frame)
                .expect("FreeWorld process address-space root teardown failed");
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameReuseStats {
    pub available: usize,
    pub returned_total: u64,
    pub reused_total: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryCachePolicy {
    Normal,
    Device,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PagePermissions {
    writable: bool,
    executable: bool,
    user: bool,
    cache: MemoryCachePolicy,
}

impl PagePermissions {
    pub const fn read_only() -> Self {
        Self {
            writable: false,
            executable: false,
            user: false,
            cache: MemoryCachePolicy::Normal,
        }
    }

    pub const fn read_write() -> Self {
        Self {
            writable: true,
            executable: false,
            user: false,
            cache: MemoryCachePolicy::Normal,
        }
    }

    pub const fn read_execute() -> Self {
        Self {
            writable: false,
            executable: true,
            user: false,
            cache: MemoryCachePolicy::Normal,
        }
    }

    pub const fn user_read_write() -> Self {
        Self {
            writable: true,
            executable: false,
            user: true,
            cache: MemoryCachePolicy::Normal,
        }
    }

    pub const fn user_read_execute() -> Self {
        Self {
            writable: false,
            executable: true,
            user: true,
            cache: MemoryCachePolicy::Normal,
        }
    }

    pub const fn new(
        writable: bool,
        executable: bool,
        user: bool,
    ) -> Result<Self, MemoryError> {
        Self::new_with_cache(
            writable,
            executable,
            user,
            MemoryCachePolicy::Normal,
        )
    }

    pub const fn new_with_cache(
        writable: bool,
        executable: bool,
        user: bool,
        cache: MemoryCachePolicy,
    ) -> Result<Self, MemoryError> {
        if writable && executable {
            return Err(MemoryError::WriteExecuteDenied);
        }
        if matches!(cache, MemoryCachePolicy::Device) && executable {
            return Err(MemoryError::ExecutableDeviceMappingDenied);
        }

        Ok(Self {
            writable,
            executable,
            user,
            cache,
        })
    }

    pub const fn device_read_write() -> Self {
        Self {
            writable: true,
            executable: false,
            user: false,
            cache: MemoryCachePolicy::Device,
        }
    }

    pub const fn writable(self) -> bool {
        self.writable
    }

    pub const fn executable(self) -> bool {
        self.executable
    }

    pub const fn user(self) -> bool {
        self.user
    }

    pub const fn cache_policy(self) -> MemoryCachePolicy {
        self.cache
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryError {
    AlreadyInitialized,
    NotInitialized,
    NoPhysicalMemoryMapping,
    InvalidVirtualAddress,
    AddressNotAligned,
    InvalidPhysicalFrame,
    FrameNotAllocatorOwned,
    FrameReservedByAllocator,
    FrameStateBitmapTooLarge,
    OutOfFrames,
    PageAlreadyMapped,
    PageNotMapped,
    ParentHugePage,
    InvalidFrameAddress,
    WriteExecuteDenied,
    ExecutableDeviceMappingDenied,
    DirectMapUnexpectedPageSize,
    PageAttributeTableUnsupported,
    DeviceCachePolicyUnavailable,
    NoSelfTestVirtualAddress,
    SelfTestDataMismatch,
    SelfTestFrameMismatch,
    VirtualPageReserved,
    VirtualPageNotReserved,
    VirtualReservationTableFull,
    KernelMappingOutsideHigherHalf,
    UserMappingOutsideLowerHalf,
    BootAddressSpaceViolation,
    InvalidAddressSpaceRoot,
    AddressSpaceRootActive,
    AddressSpaceRootNotEmpty,
    InvalidProcessUserPermissions,
    ProcessUserLeafAlreadyMapped,
    ProcessUserLeafCorrupt,
}

pub fn allocate_frame() -> Result<PhysFrame, MemoryError> {
    crate::arch::memory::allocate_frame()
}

/// Returns a physical frame to FreeWorld's recycler.
///
/// # Safety
///
/// The caller must own the frame exclusively and must prove that no
/// owner-visible virtual mapping, raw pointer, DMA mapping, device, or other
/// consumer can still use it. The allocator's own privileged physical-memory
/// mapping is excluded from that condition because the recycler uses it as
/// metadata access. After this call succeeds, the frame's contents are
/// destroyed and the caller must never access it again unless it is later
/// returned by `allocate_frame()`.
pub unsafe fn free_frame(frame: PhysFrame) -> Result<(), MemoryError> {
    unsafe { crate::arch::memory::free_frame(frame) }
}

pub fn frame_reuse_stats() -> Result<FrameReuseStats, MemoryError> {
    crate::arch::memory::frame_reuse_stats()
}

/// Reserves one virtual page against generic map_page() callers.
///
/// C2b uses this for live task-stack guard pages. Reservations are fixed-size,
/// allocation-free metadata and are independent of whether a leaf PTE exists.
/// M3.5-C remains single-CPU. The current check-then-claim reservation path
/// is not sufficient for two CPUs racing to reserve the same address; SMP must
/// replace or serialize it before multiple CPUs mutate mappings concurrently.
pub fn reserve_virtual_page(virtual_address: u64) -> Result<(), MemoryError> {
    if virtual_address == 0 || virtual_address % PAGE_SIZE != 0 {
        return Err(if virtual_address % PAGE_SIZE != 0 {
            MemoryError::AddressNotAligned
        } else {
            MemoryError::InvalidVirtualAddress
        });
    }

    for slot in &RESERVED_VIRTUAL_PAGES {
        if slot.load(Ordering::Acquire) == virtual_address {
            return Err(MemoryError::VirtualPageReserved);
        }
    }

    for slot in &RESERVED_VIRTUAL_PAGES {
        if slot
            .compare_exchange(0, virtual_address, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Ok(());
        }
    }

    Err(MemoryError::VirtualReservationTableFull)
}

pub fn release_virtual_page_reservation(
    virtual_address: u64,
) -> Result<(), MemoryError> {
    if virtual_address == 0 {
        return Err(MemoryError::InvalidVirtualAddress);
    }
    if virtual_address % PAGE_SIZE != 0 {
        return Err(MemoryError::AddressNotAligned);
    }

    for slot in &RESERVED_VIRTUAL_PAGES {
        if slot
            .compare_exchange(virtual_address, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Ok(());
        }
    }

    Err(MemoryError::VirtualPageNotReserved)
}

pub fn is_virtual_page_reserved(virtual_address: u64) -> bool {
    RESERVED_VIRTUAL_PAGES
        .iter()
        .any(|slot| slot.load(Ordering::Acquire) == virtual_address)
}

#[cfg(feature = "m35a-ci-self-test")]
fn frame_is_allocated_for_test(frame: PhysFrame) -> Result<bool, MemoryError> {
    crate::arch::memory::frame_is_allocated_for_test(frame)
}

/// Maps a single 4 KiB page.
///
/// # Safety
///
/// The caller must ensure that the virtual page is unused and that creating
/// this mapping cannot violate Rust aliasing or object-validity rules.
pub unsafe fn map_page(
    virtual_address: u64,
    frame: PhysFrame,
    permissions: PagePermissions,
) -> Result<(), MemoryError> {
    if permissions.writable() && permissions.executable() {
        return Err(MemoryError::WriteExecuteDenied);
    }
    address_space::validate_mapping_target(virtual_address, permissions)?;
    if is_virtual_page_reserved(virtual_address) {
        return Err(MemoryError::VirtualPageReserved);
    }

    unsafe { crate::arch::memory::map_page(virtual_address, frame, permissions) }
}

/// Removes a single 4 KiB mapping and returns the physical frame that was mapped.
///
/// Unmapping does not automatically recycle the frame. The returned frame remains
/// owned by the caller. M3.5-A callers that can prove all other aliases are gone
/// may transfer that ownership to `free_frame()`.
pub fn unmap_page(virtual_address: u64) -> Result<PhysFrame, MemoryError> {
    crate::arch::memory::unmap_page(virtual_address)
}

#[cfg(feature = "m1-ci-self-test")]
pub fn ci_self_test() -> Result<(), MemoryError> {
    const TEST_VIRTUAL_ADDRESSES: [u64; 4] = [
        0xffff_e100_0000_0000,
        0xffff_e200_0000_0000,
        0xffff_e300_0000_0000,
        0xffff_e400_0000_0000,
    ];
    const TEST_PATTERN: u64 = 0x4657_4f53_4d31_5445;

    if PagePermissions::new(true, true, false)
        != Err(MemoryError::WriteExecuteDenied)
    {
        return Err(MemoryError::WriteExecuteDenied);
    }

    let frame = allocate_frame()?;
    let permissions = PagePermissions::read_write();

    let mut mapped_address = None;
    for address in TEST_VIRTUAL_ADDRESSES {
        match unsafe { map_page(address, frame, permissions) } {
            Ok(()) => {
                mapped_address = Some(address);
                break;
            }
            Err(MemoryError::PageAlreadyMapped) => continue,
            Err(error) => return Err(error),
        }
    }

    let address = mapped_address.ok_or(MemoryError::NoSelfTestVirtualAddress)?;

    // SAFETY: The page was just mapped RW and is reserved exclusively for this
    // self-test. Volatile raw accesses avoid creating aliased Rust references.
    let observed = unsafe {
        let pointer = address as *mut u64;
        core::ptr::write_volatile(pointer, TEST_PATTERN);
        core::ptr::read_volatile(pointer)
    };

    if observed != TEST_PATTERN {
        return Err(MemoryError::SelfTestDataMismatch);
    }

    let unmapped = unmap_page(address)?;
    if unmapped != frame {
        return Err(MemoryError::SelfTestFrameMismatch);
    }

    Ok(())
}


#[cfg(feature = "m35a-ci-self-test")]
pub fn frame_reuse_ci_self_test() -> Result<(), MemoryError> {
    const TEST_VIRTUAL_ADDRESSES: [u64; 4] = [
        0xffff_e500_0000_0000,
        0xffff_e600_0000_0000,
        0xffff_e700_0000_0000,
        0xffff_e800_0000_0000,
    ];
    const FIRST_PATTERN: u64 = 0x4657_4f53_4652_4545;
    const SECOND_PATTERN: u64 = 0x4657_4f53_5245_5553;

    let before = frame_reuse_stats()?;

    if unsafe { free_frame(PhysFrame { start: 0 }) }
        != Err(MemoryError::FrameNotAllocatorOwned)
    {
        return Err(MemoryError::SelfTestFrameMismatch);
    }

    let frame = allocate_frame()?;
    if !frame_is_allocated_for_test(frame)? {
        return Err(MemoryError::SelfTestFrameMismatch);
    }

    let permissions = PagePermissions::read_write();

    let mut mapped_address = None;
    for address in TEST_VIRTUAL_ADDRESSES {
        match unsafe { map_page(address, frame, permissions) } {
            Ok(()) => {
                mapped_address = Some(address);
                break;
            }
            Err(MemoryError::PageAlreadyMapped) => continue,
            Err(error) => return Err(error),
        }
    }

    let address = mapped_address.ok_or(MemoryError::NoSelfTestVirtualAddress)?;

    unsafe {
        core::ptr::write_volatile(address as *mut u64, FIRST_PATTERN);
    }

    let unmapped = unmap_page(address)?;
    if unmapped != frame {
        return Err(MemoryError::SelfTestFrameMismatch);
    }

    // SAFETY: The only test mapping was removed above, the self-test owns the
    // frame exclusively, and no device/DMA consumer was ever given the frame.
    unsafe { free_frame(unmapped)? };

    if frame_is_allocated_for_test(frame)? {
        return Err(MemoryError::SelfTestFrameMismatch);
    }

    let after_free = frame_reuse_stats()?;
    if after_free.available != before.available + 1
        || after_free.returned_total != before.returned_total + 1
        || after_free.reused_total != before.reused_total
    {
        return Err(MemoryError::SelfTestFrameMismatch);
    }

    let reused = allocate_frame()?;
    if reused != frame || !frame_is_allocated_for_test(reused)? {
        return Err(MemoryError::SelfTestFrameMismatch);
    }

    let after_reuse = frame_reuse_stats()?;
    if after_reuse.available != before.available
        || after_reuse.returned_total != before.returned_total + 1
        || after_reuse.reused_total != before.reused_total + 1
    {
        return Err(MemoryError::SelfTestFrameMismatch);
    }

    unsafe { map_page(address, reused, permissions)? };

    // Recycled frames are scrubbed before reuse. The intrusive free-stack link
    // and stale data from the previous owner must both be gone.
    let scrubbed = unsafe { core::ptr::read_volatile(address as *const u64) };
    if scrubbed != 0 {
        return Err(MemoryError::SelfTestDataMismatch);
    }

    unsafe {
        core::ptr::write_volatile(address as *mut u64, SECOND_PATTERN);
    }

    let returned_again = unmap_page(address)?;
    if returned_again != reused {
        return Err(MemoryError::SelfTestFrameMismatch);
    }

    // Leave the successfully-tested frame in the recycler so later bootstrap
    // allocations can consume it instead of leaking the self-test frame.
    unsafe { free_frame(returned_again)? };

    if frame_is_allocated_for_test(returned_again)? {
        return Err(MemoryError::SelfTestFrameMismatch);
    }

    Ok(())
}

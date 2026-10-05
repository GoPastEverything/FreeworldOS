pub mod heap;

pub const PAGE_SIZE: u64 = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysFrame {
    pub start: u64,
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
        0x0000_4000_0000_0000,
        0x0000_5000_0000_0000,
        0x0000_6000_0000_0000,
        0x0000_7000_0000_0000,
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
        0x0000_2000_0000_0000,
        0x0000_2100_0000_0000,
        0x0000_2200_0000_0000,
        0x0000_2300_0000_0000,
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

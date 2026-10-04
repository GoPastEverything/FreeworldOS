pub const PAGE_SIZE: u64 = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysFrame {
    pub start: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PagePermissions {
    writable: bool,
    executable: bool,
    user: bool,
}

impl PagePermissions {
    pub const fn read_only() -> Self {
        Self {
            writable: false,
            executable: false,
            user: false,
        }
    }

    pub const fn read_write() -> Self {
        Self {
            writable: true,
            executable: false,
            user: false,
        }
    }

    pub const fn read_execute() -> Self {
        Self {
            writable: false,
            executable: true,
            user: false,
        }
    }

    pub const fn user_read_write() -> Self {
        Self {
            writable: true,
            executable: false,
            user: true,
        }
    }

    pub const fn user_read_execute() -> Self {
        Self {
            writable: false,
            executable: true,
            user: true,
        }
    }

    pub const fn new(
        writable: bool,
        executable: bool,
        user: bool,
    ) -> Result<Self, MemoryError> {
        if writable && executable {
            return Err(MemoryError::WriteExecuteDenied);
        }

        Ok(Self {
            writable,
            executable,
            user,
        })
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryError {
    AlreadyInitialized,
    NotInitialized,
    NoPhysicalMemoryMapping,
    InvalidVirtualAddress,
    AddressNotAligned,
    InvalidPhysicalFrame,
    OutOfFrames,
    PageAlreadyMapped,
    PageNotMapped,
    ParentHugePage,
    InvalidFrameAddress,
    WriteExecuteDenied,
    NoSelfTestVirtualAddress,
    SelfTestDataMismatch,
    SelfTestFrameMismatch,
}

pub fn allocate_frame() -> Result<PhysFrame, MemoryError> {
    crate::arch::memory::allocate_frame()
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
/// M1 has no frame deallocator. The returned frame is still owned by the caller
/// and must not be treated as automatically reusable or returned to a free pool.
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

use bootloader_api::BootInfo;

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
}

pub fn init(boot_info: &'static mut BootInfo) -> Result<(), MemoryError> {
    crate::arch::memory::init(boot_info)
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

pub fn unmap_page(virtual_address: u64) -> Result<PhysFrame, MemoryError> {
    crate::arch::memory::unmap_page(virtual_address)
}

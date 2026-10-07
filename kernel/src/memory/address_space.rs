use bootloader_api::BootInfo;

use super::{MemoryError, PagePermissions, PAGE_SIZE};

pub const USER_ADDRESS_START: u64 = PAGE_SIZE;
pub const USER_ADDRESS_END_EXCLUSIVE: u64 = 0x0000_8000_0000_0000;
pub const KERNEL_ADDRESS_START: u64 = 0xffff_8000_0000_0000;

pub const BOOT_KERNEL_IMAGE_BASE: u64 = 0xffff_8100_0000_0000;
pub const BOOT_KERNEL_STACK_GUARD_BASE: u64 = 0xffff_8200_0000_0000;
pub const BOOT_INFO_BASE: u64 = 0xffff_8300_0000_0000;
pub const BOOT_PHYSICAL_MEMORY_BASE: u64 = 0xffff_c000_0000_0000;
pub const BOOT_DYNAMIC_START: u64 = 0xffff_d000_0000_0000;
pub const BOOT_DYNAMIC_END: u64 = 0xffff_dfff_ffff_f000;

pub const KERNEL_SELF_TEST_START: u64 = 0xffff_e000_0000_0000;
pub const USER_SELF_TEST_START: u64 = 0x0000_4000_0000_0000;

pub const fn is_user_address(address: u64) -> bool {
    address >= USER_ADDRESS_START && address < USER_ADDRESS_END_EXCLUSIVE
}

pub const fn is_kernel_address(address: u64) -> bool {
    address >= KERNEL_ADDRESS_START
}

pub const fn validate_mapping_target(
    virtual_address: u64,
    permissions: PagePermissions,
) -> Result<(), MemoryError> {
    if permissions.user() {
        if !is_user_address(virtual_address) {
            return Err(MemoryError::UserMappingOutsideLowerHalf);
        }
    } else if !is_kernel_address(virtual_address) {
        return Err(MemoryError::KernelMappingOutsideHigherHalf);
    }

    Ok(())
}

pub fn validate_boot_layout(
    boot_info: &BootInfo,
    physical_memory_offset: u64,
) -> Result<(), MemoryError> {
    let boot_info_address = boot_info as *const BootInfo as u64;

    if boot_info.kernel_image_offset != BOOT_KERNEL_IMAGE_BASE
        || !range_in_kernel_half(
            boot_info.kernel_image_offset,
            boot_info.kernel_len,
        )
    {
        return Err(MemoryError::BootAddressSpaceViolation);
    }

    let expected_stack_bottom = BOOT_KERNEL_STACK_GUARD_BASE
        .checked_add(PAGE_SIZE)
        .ok_or(MemoryError::BootAddressSpaceViolation)?;
    if boot_info.kernel_stack_bottom != expected_stack_bottom
        || !range_in_kernel_half(
            boot_info.kernel_stack_bottom,
            boot_info.kernel_stack_len,
        )
    {
        return Err(MemoryError::BootAddressSpaceViolation);
    }

    if boot_info_address != BOOT_INFO_BASE
        || !is_kernel_address(boot_info_address)
    {
        return Err(MemoryError::BootAddressSpaceViolation);
    }

    if physical_memory_offset != BOOT_PHYSICAL_MEMORY_BASE
        || !is_kernel_address(physical_memory_offset)
    {
        return Err(MemoryError::BootAddressSpaceViolation);
    }

    Ok(())
}

const fn range_in_kernel_half(start: u64, length: u64) -> bool {
    if !is_kernel_address(start) {
        return false;
    }

    if length == 0 {
        return true;
    }

    match start.checked_add(length - 1) {
        Some(end) => is_kernel_address(end),
        None => false,
    }
}

#[cfg(feature = "m5a-ci-self-test")]
pub fn ci_self_test() -> Result<(), MemoryError> {
    const USER_TEST_PAGE: u64 = USER_SELF_TEST_START;
    const KERNEL_TEST_PAGE: u64 = KERNEL_SELF_TEST_START;
    const USER_PATTERN: u64 = 0x4d35_4155_5345_5250;
    const KERNEL_PATTERN: u64 = 0x4d35_414b_4552_4e50;

    let frame = super::allocate_frame()?;

    if unsafe {
        super::map_page(
            USER_TEST_PAGE,
            frame,
            PagePermissions::read_write(),
        )
    } != Err(MemoryError::KernelMappingOutsideHigherHalf)
    {
        return Err(MemoryError::KernelMappingOutsideHigherHalf);
    }

    if unsafe {
        super::map_page(
            0,
            frame,
            PagePermissions::user_read_write(),
        )
    } != Err(MemoryError::UserMappingOutsideLowerHalf)
    {
        return Err(MemoryError::UserMappingOutsideLowerHalf);
    }

    if unsafe {
        super::map_page(
            KERNEL_TEST_PAGE,
            frame,
            PagePermissions::user_read_write(),
        )
    } != Err(MemoryError::UserMappingOutsideLowerHalf)
    {
        return Err(MemoryError::UserMappingOutsideLowerHalf);
    }

    unsafe {
        super::map_page(
            USER_TEST_PAGE,
            frame,
            PagePermissions::user_read_write(),
        )?;
        core::ptr::write_volatile(USER_TEST_PAGE as *mut u64, USER_PATTERN);
    }

    if unsafe { core::ptr::read_volatile(USER_TEST_PAGE as *const u64) }
        != USER_PATTERN
    {
        return Err(MemoryError::SelfTestDataMismatch);
    }

    let user_unmapped = super::unmap_page(USER_TEST_PAGE)?;
    if user_unmapped != frame {
        return Err(MemoryError::SelfTestFrameMismatch);
    }

    unsafe {
        super::map_page(
            KERNEL_TEST_PAGE,
            frame,
            PagePermissions::read_write(),
        )?;
        core::ptr::write_volatile(
            KERNEL_TEST_PAGE as *mut u64,
            KERNEL_PATTERN,
        );
    }

    if unsafe { core::ptr::read_volatile(KERNEL_TEST_PAGE as *const u64) }
        != KERNEL_PATTERN
    {
        return Err(MemoryError::SelfTestDataMismatch);
    }

    let kernel_unmapped = super::unmap_page(KERNEL_TEST_PAGE)?;
    if kernel_unmapped != frame {
        return Err(MemoryError::SelfTestFrameMismatch);
    }

    // SAFETY: both temporary mappings were removed and this self-test still
    // owns the physical frame exclusively.
    unsafe {
        super::free_frame(kernel_unmapped)?;
    }

    crate::arch::serial::println(
        "FreeWorldOS: M5-A address-space self-test: passed boot_mappings=higher kernel_lower=refused user_higher=refused user_lower=ok kernel_higher=ok null_user=reserved",
    );

    Ok(())
}

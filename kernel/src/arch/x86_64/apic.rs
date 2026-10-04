use core::{
    arch::x86_64::__cpuid,
    ptr::{read_volatile, write_volatile},
    sync::atomic::{AtomicU64, Ordering},
};

use x86_64::{
    registers::model_specific::{ApicBase, ApicBaseFlags},
    VirtAddr,
};

use super::serial;

pub const TIMER_VECTOR: u8 = 0xE0;
pub const SPURIOUS_VECTOR: u8 = 0xFF;

const CPUID_FEATURE_APIC: u32 = 1 << 9;

const REG_ID: u64 = 0x020;
const REG_VERSION: u64 = 0x030;
const REG_TASK_PRIORITY: u64 = 0x080;
const REG_EOI: u64 = 0x0B0;
const REG_SPURIOUS: u64 = 0x0F0;
const REG_LVT_TIMER: u64 = 0x320;
const REG_TIMER_INITIAL_COUNT: u64 = 0x380;
const REG_TIMER_DIVIDE_CONFIG: u64 = 0x3E0;

const SPURIOUS_SOFTWARE_ENABLE: u32 = 1 << 8;
const LVT_MASKED: u32 = 1 << 16;
const TIMER_DIVIDE_BY_16: u32 = 0x3;

static APIC_BASE_VIRTUAL: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApicError {
    Unsupported,
    X2ApicAlreadyActive,
    VirtualAddressOverflow,
    InvalidVirtualAddress,
}

pub fn init(physical_offset: u64) -> Result<(), ApicError> {
    let features = __cpuid(1);
    if features.edx & CPUID_FEATURE_APIC == 0 {
        return Err(ApicError::Unsupported);
    }

    let (frame, mut flags) = ApicBase::read();
    if flags.contains(ApicBaseFlags::X2APIC_ENABLE) {
        // M2 intentionally brings up the MMIO xAPIC path first. Do not silently
        // change APIC modes underneath firmware; x2APIC support can be added as
        // a separate backend later.
        return Err(ApicError::X2ApicAlreadyActive);
    }

    if !flags.contains(ApicBaseFlags::LAPIC_ENABLE) {
        flags.insert(ApicBaseFlags::LAPIC_ENABLE);
        // SAFETY: We preserve the firmware-selected APIC frame and only enable
        // the LAPIC bit after CPUID confirmed APIC support.
        unsafe { ApicBase::write(frame, flags) };
    }

    let physical_base = frame.start_address().as_u64();
    let virtual_base_raw = physical_offset
        .checked_add(physical_base)
        .ok_or(ApicError::VirtualAddressOverflow)?;
    let virtual_base =
        VirtAddr::try_new(virtual_base_raw).map_err(|_| ApicError::InvalidVirtualAddress)?;

    APIC_BASE_VIRTUAL.store(virtual_base.as_u64(), Ordering::Release);

    // SAFETY: The bootloader's physical-memory mapping covers the xAPIC MMIO
    // frame on our current x86_64 bootstrap. Accesses are volatile 32-bit APIC
    // register accesses and IF is still clear during initialization.
    unsafe {
        write(REG_TASK_PRIORITY, 0);

        let current_spurious = read(REG_SPURIOUS);
        let new_spurious = (current_spurious & !0xFF)
            | u32::from(SPURIOUS_VECTOR)
            | SPURIOUS_SOFTWARE_ENABLE;
        write(REG_SPURIOUS, new_spurious);

        // The timer exists and has an IDT vector, but remains masked and stopped
        // until the calibration commit measures it against a reference clock.
        write(
            REG_LVT_TIMER,
            u32::from(TIMER_VECTOR) | LVT_MASKED,
        );
        write(REG_TIMER_DIVIDE_CONFIG, TIMER_DIVIDE_BY_16);
        write(REG_TIMER_INITIAL_COUNT, 0);

        let id = read(REG_ID) >> 24;
        let version = read(REG_VERSION) & 0xFF;
        serial::write_fmt(format_args!(
            "  apic: xAPIC base={physical_base:#x} id={id:#x} version={version:#x} spurious={SPURIOUS_VECTOR:#x} timer={TIMER_VECTOR:#x} masked\n"
        ));
    }

    Ok(())
}

pub fn eoi() {
    let base = APIC_BASE_VIRTUAL.load(Ordering::Acquire);
    if base == 0 {
        return;
    }

    // SAFETY: A nonzero base is published only after successful LAPIC
    // initialization, and EOI is a write-only 32-bit xAPIC register.
    unsafe {
        let pointer = (base + REG_EOI) as *mut u32;
        write_volatile(pointer, 0);
    }
}

unsafe fn read(offset: u64) -> u32 {
    let base = APIC_BASE_VIRTUAL.load(Ordering::Acquire);
    // SAFETY: The caller guarantees successful APIC initialization and a valid
    // register offset.
    unsafe { read_volatile((base + offset) as *const u32) }
}

unsafe fn write(offset: u64, value: u32) {
    let base = APIC_BASE_VIRTUAL.load(Ordering::Acquire);
    // SAFETY: The caller guarantees successful APIC initialization and a valid
    // register offset.
    unsafe { write_volatile((base + offset) as *mut u32, value) };
}

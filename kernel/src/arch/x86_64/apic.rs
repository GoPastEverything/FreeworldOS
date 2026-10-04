use core::{
    arch::x86_64::__cpuid,
    ptr::{read_volatile, write_volatile},
    sync::atomic::{AtomicU64, Ordering},
};

use x86_64::registers::model_specific::{ApicBase, ApicBaseFlags};

use crate::memory::{MemoryError, PagePermissions, PhysFrame};

use super::{memory, pit, serial};

pub const TIMER_VECTOR: u8 = 0xE0;
pub const SPURIOUS_VECTOR: u8 = 0xFF;

const CPUID_FEATURE_APIC: u32 = 1 << 9;

const REG_ID: u64 = 0x020;
const REG_VERSION: u64 = 0x030;
const REG_TASK_PRIORITY: u64 = 0x080;
const REG_EOI: u64 = 0x0B0;
const REG_SPURIOUS: u64 = 0x0F0;
const REG_LVT_CMCI: u64 = 0x2F0;
const REG_LVT_TIMER: u64 = 0x320;
const REG_LVT_THERMAL: u64 = 0x330;
const REG_LVT_PERFORMANCE: u64 = 0x340;
const REG_LVT_LINT0: u64 = 0x350;
const REG_LVT_LINT1: u64 = 0x360;
const REG_LVT_ERROR: u64 = 0x370;
const REG_TIMER_INITIAL_COUNT: u64 = 0x380;
const REG_TIMER_CURRENT_COUNT: u64 = 0x390;
const REG_TIMER_DIVIDE_CONFIG: u64 = 0x3E0;

const SPURIOUS_SOFTWARE_ENABLE: u32 = 1 << 8;
const LVT_MASKED: u32 = 1 << 16;
const LVT_TIMER_PERIODIC: u32 = 1 << 17;
const TIMER_DIVIDE_BY_16: u32 = 0x3;
const LVT_DELIVERY_NMI: u32 = 0b100 << 8;
const LAPIC_VIRTUAL_BASE: u64 = 0xffff_8000_0000_0000;

static APIC_BASE_VIRTUAL: AtomicU64 = AtomicU64::new(0);
static APIC_TIMER_HZ: AtomicU64 = AtomicU64::new(0);
static APIC_TIMER_PERIOD_NS: AtomicU64 = AtomicU64::new(0);
static APIC_TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApicError {
    Unsupported,
    X2ApicAlreadyActive,
    Memory(MemoryError),
    Pit(pit::PitError),
    InsufficientLvtEntries { max_lvt: u32 },
    TimerExpiredDuringCalibration,
    TimerProducedZeroSample,
    TimerRateOverflow,
    ImplausibleTimerRate { hz: u64 },
    TimerNotCalibrated,
    InvalidPeriodicRate { hz: u64 },
    TimerInitialCountOverflow { count: u64 },
}


#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerCalibration {
    pub counter_hz: u64,
    pub source_hz_estimate: u64,
    pub sample_ticks: [u32; 5],
}

pub fn init() -> Result<(), ApicError> {
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

    // If the bootloader direct map reaches this MMIO region, give its containing
    // huge page the same uncached policy so we do not retain a conflicting
    // cacheable alias. If it does not reach the LAPIC, there is no alias to fix.
    let direct_alias_hardened =
        memory::harden_direct_map_device_alias(physical_base)
            .map_err(ApicError::Memory)?;

    // SAFETY: This dedicated kernel virtual page is reserved for the LAPIC,
    // maps the firmware-selected 4 KiB APIC frame, is supervisor-only, NX, and
    // uses FreeWorld's Device/uncached cache policy.
    unsafe {
        crate::memory::map_page(
            LAPIC_VIRTUAL_BASE,
            PhysFrame { start: physical_base },
            PagePermissions::device_read_write(),
        )
    }
    .map_err(ApicError::Memory)?;

    APIC_BASE_VIRTUAL.store(LAPIC_VIRTUAL_BASE, Ordering::Release);

    // SAFETY: APIC_BASE_VIRTUAL now points at the dedicated uncached mapping.
    // Accesses are volatile 32-bit APIC register operations and IF is clear.
    unsafe {
        write(REG_TASK_PRIORITY, 0);

        let current_spurious = read(REG_SPURIOUS);
        let new_spurious = (current_spurious & !0xFF)
            | u32::from(SPURIOUS_VECTOR)
            | SPURIOUS_SOFTWARE_ENABLE;
        write(REG_SPURIOUS, new_spurious);

        let version_register = read(REG_VERSION);
        let version = version_register & 0xFF;
        let max_lvt = (version_register >> 16) & 0xFF;

        // Start from a deterministic LAPIC input state rather than inheriting
        // firmware programming. LINT1 is explicitly the NMI input; other
        // non-timer local sources are masked until FreeWorld owns them.
        if max_lvt < 4 {
            return Err(ApicError::InsufficientLvtEntries { max_lvt });
        }
        // max_lvt is the highest implemented LVT index, not a count.
        // Timer/LINT0/LINT1/error exist on the baseline APIC we accept.
        write(REG_LVT_LINT0, LVT_MASKED);
        write(REG_LVT_LINT1, LVT_DELIVERY_NMI);
        write(REG_LVT_ERROR, LVT_MASKED);

        if max_lvt >= 4 {
            write(REG_LVT_PERFORMANCE, LVT_MASKED);
        }
        if max_lvt >= 5 {
            write(REG_LVT_THERMAL, LVT_MASKED);
        }
        if max_lvt >= 6 {
            write(REG_LVT_CMCI, LVT_MASKED);
        }

        // The timer exists and has an IDT vector, but remains masked and stopped
        // until the calibration commit measures it against a reference clock.
        write(REG_LVT_TIMER, u32::from(TIMER_VECTOR) | LVT_MASKED);
        write(REG_TIMER_DIVIDE_CONFIG, TIMER_DIVIDE_BY_16);
        write(REG_TIMER_INITIAL_COUNT, 0);

        let id = read(REG_ID) >> 24;
        serial::write_fmt(format_args!(
            "  apic: xAPIC phys={physical_base:#x} virt={LAPIC_VIRTUAL_BASE:#x} id={id:#x} version={version:#x} max_lvt={max_lvt} spurious={SPURIOUS_VECTOR:#x} timer={TIMER_VECTOR:#x} masked direct_alias_uc={direct_alias_hardened}\n"
        ));
    }

    Ok(())
}


pub fn calibrate_timer_against_pit() -> Result<TimerCalibration, ApicError> {
    const SAMPLE_COUNT: usize = 5;
    const MIN_PLAUSIBLE_HZ: u64 = 1_000;
    const MAX_PLAUSIBLE_HZ: u64 = 2_000_000_000;

    let mut sample_ticks = [0u32; SAMPLE_COUNT];
    let mut sample_hz = [0u64; SAMPLE_COUNT];

    for index in 0..SAMPLE_COUNT {
        let window = pit::prepare_channel2_window(pit::CALIBRATION_COUNT)
            .map_err(ApicError::Pit)?;

        // SAFETY: The LAPIC is initialized, the timer LVT remains masked, and
        // IF is still clear. A maximum one-shot count lets us measure elapsed
        // timer clocks without any interrupt delivery.
        unsafe {
            write(REG_LVT_TIMER, u32::from(TIMER_VECTOR) | LVT_MASKED);
            write(REG_TIMER_DIVIDE_CONFIG, TIMER_DIVIDE_BY_16);
            write(REG_TIMER_INITIAL_COUNT, u32::MAX);
        }

        pit::run_prepared_window(window).map_err(ApicError::Pit)?;

        let current = unsafe { read(REG_TIMER_CURRENT_COUNT) };
        unsafe { write(REG_TIMER_INITIAL_COUNT, 0) };

        if current == 0 {
            return Err(ApicError::TimerExpiredDuringCalibration);
        }

        let elapsed = u32::MAX - current;
        if elapsed == 0 {
            return Err(ApicError::TimerProducedZeroSample);
        }

        let hz = u64::from(elapsed)
            .checked_mul(pit::PIT_HZ)
            .ok_or(ApicError::TimerRateOverflow)?
            / u64::from(pit::CALIBRATION_COUNT);

        sample_ticks[index] = elapsed;
        sample_hz[index] = hz;
    }

    sample_hz.sort_unstable();
    let counter_hz = sample_hz[SAMPLE_COUNT / 2];

    if !(MIN_PLAUSIBLE_HZ..=MAX_PLAUSIBLE_HZ).contains(&counter_hz) {
        return Err(ApicError::ImplausibleTimerRate { hz: counter_hz });
    }

    let source_hz_estimate = counter_hz
        .checked_mul(16)
        .ok_or(ApicError::TimerRateOverflow)?;

    APIC_TIMER_HZ.store(counter_hz, Ordering::Release);

    serial::write_fmt(format_args!(
        "  apic: timer calibration pit_hz={} window_count={} samples={sample_ticks:?} median_hz={counter_hz} source_hz_est={source_hz_estimate}\n",
        pit::PIT_HZ,
        pit::CALIBRATION_COUNT,
    ));

    Ok(TimerCalibration {
        counter_hz,
        source_hz_estimate,
        sample_ticks,
    })
}

pub fn timer_counter_hz() -> Option<u64> {
    let hz = APIC_TIMER_HZ.load(Ordering::Acquire);
    (hz != 0).then_some(hz)
}


pub fn program_periodic_timer(target_hz: u64) -> Result<u32, ApicError> {
    if target_hz == 0 {
        return Err(ApicError::InvalidPeriodicRate { hz: target_hz });
    }

    let counter_hz = timer_counter_hz().ok_or(ApicError::TimerNotCalibrated)?;
    let count = counter_hz / target_hz;
    if count == 0 {
        return Err(ApicError::InvalidPeriodicRate { hz: target_hz });
    }

    let initial_count = u32::try_from(count)
        .map_err(|_| ApicError::TimerInitialCountOverflow { count })?;

    let period_ns = count
        .checked_mul(1_000_000_000)
        .ok_or(ApicError::TimerRateOverflow)?
        / counter_hz;

    APIC_TIMER_PERIOD_NS.store(period_ns, Ordering::Release);
    APIC_TIMER_TICKS.store(0, Ordering::Release);

    // SAFETY: Calibration has completed with IF clear. The handler and IDT
    // vector are already installed. This programs only the local APIC timer;
    // interrupt delivery cannot begin until the later explicit STI.
    unsafe {
        write(REG_TIMER_DIVIDE_CONFIG, TIMER_DIVIDE_BY_16);
        write(
            REG_LVT_TIMER,
            u32::from(TIMER_VECTOR) | LVT_TIMER_PERIODIC,
        );
        write(REG_TIMER_INITIAL_COUNT, initial_count);
    }

    Ok(initial_count)
}

pub fn timer_interrupt() {
    APIC_TIMER_TICKS.fetch_add(1, Ordering::AcqRel);
    eoi();
}

pub fn timer_ticks() -> u64 {
    APIC_TIMER_TICKS.load(Ordering::Acquire)
}

pub fn timer_period_ns() -> Option<u64> {
    let period_ns = APIC_TIMER_PERIOD_NS.load(Ordering::Acquire);
    (period_ns != 0).then_some(period_ns)
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

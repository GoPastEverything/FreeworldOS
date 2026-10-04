use x86_64::instructions::interrupts;

use super::{apic, pic, serial};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterruptControllerError {
    InterruptsAlreadyEnabled,
    Pic(pic::PicError),
    Apic(apic::ApicError),
}

pub fn init() -> Result<(), InterruptControllerError> {
    if interrupts::are_enabled() {
        return Err(InterruptControllerError::InterruptsAlreadyEnabled);
    }

    pic::remap_and_mask().map_err(InterruptControllerError::Pic)?;
    serial::write_fmt(format_args!(
        "  pic: remapped master={:#x} slave={:#x} all IRQs masked\n",
        pic::MASTER_VECTOR_OFFSET,
        pic::SLAVE_VECTOR_OFFSET
    ));

    apic::init().map_err(InterruptControllerError::Apic)?;

    // M2 controller bring-up is deliberately pre-STI. A later commit whose
    // sole job is enabling interrupts will prove timer delivery and tick count.
    debug_assert!(!interrupts::are_enabled());
    serial::println("  irq: local APIC online; IF remains clear");

    Ok(())
}


pub fn calibrate_timer() -> Result<apic::TimerCalibration, InterruptControllerError> {
    if interrupts::are_enabled() {
        return Err(InterruptControllerError::InterruptsAlreadyEnabled);
    }

    apic::calibrate_timer_against_pit().map_err(InterruptControllerError::Apic)
}


#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerDeliveryProof {
    pub target_hz: u64,
    pub initial_count: u32,
    pub ticks_observed: u64,
}

pub fn enable_timer_delivery_and_prove(
) -> Result<TimerDeliveryProof, InterruptControllerError> {
    const TARGET_HZ: u64 = 100;
    const REQUIRED_TICKS: u64 = 3;

    if interrupts::are_enabled() {
        return Err(InterruptControllerError::InterruptsAlreadyEnabled);
    }

    let initial_count =
        apic::program_periodic_timer(TARGET_HZ).map_err(InterruptControllerError::Apic)?;

    // This is the first intentional STI in FreeWorldOS. All legacy PIC lines
    // remain masked; the LAPIC timer vector and handler are installed; the
    // timer handler performs only an atomic increment plus EOI.
    interrupts::enable();

    while apic::timer_ticks() < REQUIRED_TICKS {
        x86_64::instructions::hlt();
    }

    let ticks_observed = apic::timer_ticks();
    serial::write_fmt(format_args!(
        "  irq: APIC timer delivery proven target_hz={TARGET_HZ} initial_count={initial_count} ticks={ticks_observed} IF=on\n"
    ));

    Ok(TimerDeliveryProof {
        target_hz: TARGET_HZ,
        initial_count,
        ticks_observed,
    })
}

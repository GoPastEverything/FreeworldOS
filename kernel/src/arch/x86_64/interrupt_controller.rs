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

mod apic;
mod gdt;
mod pic;
mod pit;
pub mod exceptions;
pub mod interrupt_controller;
pub mod memory;
pub mod serial;

use bootloader_api::BootInfo;
use x86_64::instructions::interrupts;

pub fn early_init(_boot_info: &mut BootInfo) {
    interrupts::disable();
    serial::init();
    gdt::init();
    exceptions::init();
    serial::println("  arch: FreeWorld GDT/TSS/IDT online");
}

pub fn timer_ticks() -> u64 {
    apic::timer_ticks()
}

pub fn timer_period_ns() -> Option<u64> {
    apic::timer_period_ns()
}

pub fn disable_interrupts() {
    interrupts::disable();
}

pub fn halt_loop() -> ! {
    loop {
        // SAFETY: HLT is intentional while the bootstrap kernel has no runnable tasks.
        unsafe { core::arch::asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

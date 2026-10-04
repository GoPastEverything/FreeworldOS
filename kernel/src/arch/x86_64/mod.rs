mod apic;
mod gdt;
mod pic;
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

pub fn halt_loop() -> ! {
    loop {
        // SAFETY: HLT is intentional while the bootstrap kernel has no runnable tasks.
        unsafe { core::arch::asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

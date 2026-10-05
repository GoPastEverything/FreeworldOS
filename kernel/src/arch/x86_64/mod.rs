mod apic;
mod context_switch;
mod gdt;
mod interrupt_context;
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

pub unsafe fn switch_task_context(old_rsp: *mut u64, new_rsp: u64) {
    unsafe { context_switch::switch_task_context(old_rsp, new_rsp) }
}

pub unsafe fn start_first_task(new_rsp: u64) -> ! {
    unsafe { context_switch::start_first_task(new_rsp) }
}

pub fn in_interrupt() -> bool {
    interrupt_context::in_interrupt()
}

pub fn halt_loop() -> ! {
    loop {
        // SAFETY: HLT is intentional while the bootstrap kernel has no runnable tasks.
        unsafe { core::arch::asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

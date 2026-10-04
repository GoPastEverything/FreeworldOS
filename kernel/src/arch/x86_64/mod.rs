pub mod serial;

use bootloader_api::BootInfo;

pub fn early_init(_boot_info: &'static mut BootInfo) {
    serial::init();
}

pub fn halt_loop() -> ! {
    loop {
        // SAFETY: HLT is intentional while the bootstrap kernel has no runnable tasks.
        unsafe { core::arch::asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

#![no_std]
#![no_main]

mod arch;
mod exec;
mod object;
mod rt;
mod state;
mod vfs;

use bootloader_api::{entry_point, BootInfo};
use core::panic::PanicInfo;

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    arch::early_init(boot_info);
    arch::serial::println("FreeWorldOS: kernel entry");
    arch::serial::println("FreeWorldOS: x86_64 bootstrap active");

    object::init();
    vfs::init();
    state::init();
    exec::init();
    rt::init();

    arch::serial::println("FreeWorldOS: bootstrap initialization complete");
    arch::halt_loop()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    arch::serial::println("FreeWorldOS: KERNEL PANIC");
    arch::serial::write_fmt(format_args!("{info}\n"));
    arch::halt_loop()
}

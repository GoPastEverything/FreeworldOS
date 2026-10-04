#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

mod arch;
mod exec;
mod memory;
mod object;
mod rt;
mod state;
mod vfs;

use bootloader_api::{
    config::{BootloaderConfig, Mapping},
    entry_point, BootInfo,
};
use core::panic::PanicInfo;

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    arch::early_init(boot_info);
    arch::serial::println("FreeWorldOS: kernel entry");
    arch::serial::println("FreeWorldOS: x86_64 bootstrap active");

    if let Err(error) = memory::init(boot_info) {
        panic!("M1 memory initialization failed: {error:?}");
    }

    #[cfg(feature = "m1-ci-exception-test")]
    {
        arch::serial::println("FreeWorldOS: M1 exception self-test: trigger #BP");
        arch::exceptions::trigger_test_breakpoint();
        arch::serial::println("FreeWorldOS: M1 exception self-test: resumed");
    }

    object::init();
    vfs::init();
    state::init();
    exec::init();
    rt::init();

    arch::serial::println("FreeWorldOS: M1 foundation online");
    arch::serial::println("FreeWorldOS: bootstrap initialization complete");
    arch::halt_loop()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    arch::serial::println("FreeWorldOS: KERNEL PANIC");
    arch::serial::write_fmt(format_args!("{info}\n"));
    arch::halt_loop()
}

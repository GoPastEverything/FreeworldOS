#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

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

    if let Err(error) = arch::memory::init(boot_info) {
        panic!("M1 memory initialization failed: {error:?}");
    }

    if let Err(error) = memory::heap::init() {
        panic!("M3 heap initialization failed: {error:?}");
    }

    #[cfg(feature = "m3-ci-self-test")]
    {
        if let Err(error) = memory::heap::ci_self_test() {
            panic!("M3 heap self-test failed: {error:?}");
        }
        arch::serial::println("FreeWorldOS: M3 heap self-test: passed");
    }

    if let Err(error) = arch::interrupt_controller::init() {
        panic!("M2 interrupt-controller initialization failed: {error:?}");
    }

    if let Err(error) = arch::interrupt_controller::calibrate_timer() {
        panic!("M2 APIC timer calibration failed: {error:?}");
    }

    #[cfg(feature = "m1-ci-self-test")]
    {
        if let Err(error) = memory::ci_self_test() {
            panic!("M1 memory self-test failed: {error:?}");
        }
        arch::serial::println("FreeWorldOS: M1 memory self-test: passed");

        arch::serial::println("FreeWorldOS: M1 exception self-test: trigger #BP");
        arch::exceptions::trigger_test_breakpoint();
        arch::serial::println("FreeWorldOS: M1 exception self-test: resumed");
    }

    if let Err(error) = arch::interrupt_controller::enable_timer_delivery_and_prove() {
        panic!("M2 APIC timer delivery proof failed: {error:?}");
    }

    if let Err(error) = object::init() {
        panic!("M3 object initialization failed: {error:?}");
    }

    #[cfg(feature = "m3-ci-self-test")]
    {
        if let Err(error) = object::ci_self_test() {
            panic!("M3 object self-test failed: {error:?}");
        }
    }

    vfs::init();
    state::init();
    exec::init();
    rt::init();

    arch::serial::println("FreeWorldOS: M2 APIC timer delivery online; IF enabled");
    arch::serial::println("FreeWorldOS: M1 foundation online");
    arch::serial::println("FreeWorldOS: bootstrap initialization complete");
    arch::halt_loop()
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    panic!("kernel heap allocation failed: {layout:?}");
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    arch::disable_interrupts();
    arch::serial::println("FreeWorldOS: KERNEL PANIC");
    arch::serial::write_fmt(format_args!("{info}\n"));
    arch::halt_loop()
}

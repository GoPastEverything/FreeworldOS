#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

mod arch;
mod debug;
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

    config.mappings.kernel_base =
        Mapping::FixedAddress(memory::address_space::BOOT_KERNEL_IMAGE_BASE);
    config.mappings.kernel_stack =
        Mapping::FixedAddress(memory::address_space::BOOT_KERNEL_STACK_GUARD_BASE);
    config.mappings.boot_info =
        Mapping::FixedAddress(memory::address_space::BOOT_INFO_BASE);
    config.mappings.physical_memory =
        Some(Mapping::FixedAddress(
            memory::address_space::BOOT_PHYSICAL_MEMORY_BASE,
        ));

    // Any bootloader-managed mappings that remain dynamic (for example a
    // framebuffer or ramdisk mapping) are confined to the kernel half.
    config.mappings.dynamic_range_start =
        Some(memory::address_space::BOOT_DYNAMIC_START);
    config.mappings.dynamic_range_end =
        Some(memory::address_space::BOOT_DYNAMIC_END);

    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    arch::early_init(boot_info);
    debug::backtrace::init(
        boot_info.kernel_image_offset,
        boot_info.kernel_len,
    );
    debug::events::record(
        debug::events::BOOT_BEGIN,
        debug::events::SUBSYSTEM_BOOT,
        debug::events::LEVEL_INFO,
        [0, 0, 0, 0],
    );
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
        debug::selftest::run_result(
            "m3.heap",
            memory::heap::ci_self_test,
        );
        arch::serial::println("FreeWorldOS: M3 heap self-test: passed");
    }

    #[cfg(feature = "m35a-ci-self-test")]
    {
        debug::selftest::run_result(
            "m3.5a.frame_reuse",
            memory::frame_reuse_ci_self_test,
        );

        let stats = memory::frame_reuse_stats()
            .expect("M3.5-A frame reuse stats unavailable after self-test");
        arch::serial::write_fmt(format_args!(
            "FreeWorldOS: M3.5-A frame reuse self-test: passed bitmap_state=ok available={} returned_total={} reused_total={}\n",
            stats.available,
            stats.returned_total,
            stats.reused_total,
        ));
    }

    #[cfg(feature = "m5a-ci-self-test")]
    {
        debug::selftest::run_result(
            "m5a.address_space",
            memory::address_space::ci_self_test,
        );
    }

    if let Err(error) = arch::interrupt_controller::init() {
        panic!("M2 interrupt-controller initialization failed: {error:?}");
    }

    if let Err(error) = arch::interrupt_controller::calibrate_timer() {
        panic!("M2 APIC timer calibration failed: {error:?}");
    }

    #[cfg(feature = "m1-ci-self-test")]
    {
        debug::selftest::run_result(
            "m1.memory",
            memory::ci_self_test,
        );
        arch::serial::println("FreeWorldOS: M1 memory self-test: passed");

        debug::selftest::run_infallible(
            "m1.exception.breakpoint",
            || {
                arch::serial::println("FreeWorldOS: M1 exception self-test: trigger #BP");
                arch::exceptions::trigger_test_breakpoint();
                arch::serial::println("FreeWorldOS: M1 exception self-test: resumed");
            },
        );
    }

    if let Err(error) = arch::interrupt_controller::enable_timer_delivery_and_prove() {
        panic!("M2 APIC timer delivery proof failed: {error:?}");
    }

    if let Err(error) = object::init() {
        panic!("M3 object initialization failed: {error:?}");
    }

    #[cfg(feature = "m3-ci-self-test")]
    {
        debug::selftest::run_result(
            "m3.object",
            object::ci_self_test,
        );
    }

    #[cfg(feature = "m35b-ci-self-test")]
    {
        debug::selftest::run_result(
            "m3.5b.event_ring",
            debug::events::ci_self_test,
        );
        debug::events::dump_recent_to_serial(8);
    }

    #[cfg(feature = "m35c-ci-self-test")]
    {
        debug::selftest::run_result(
            "m3.5c.task_stack",
            object::task_stack_ci_self_test,
        );
    }

    #[cfg(feature = "m35c2b-ci-self-test")]
    {
        debug::selftest::run_result(
            "m3.5c.ownership",
            object::task_ownership_ci_self_test,
        );
    }

    #[cfg(feature = "m35c2c-ci-switch-test")]
    {
        rt::scheduler::ci_voluntary_switch_test();
    }

    #[cfg(feature = "m35c2d-ci-roundtrip-test")]
    {
        rt::scheduler::ci_interrupt_safe_roundtrip_test();
    }

    #[cfg(feature = "m35c2e-ci-irq-stack-test")]
    {
        rt::scheduler::ci_irq_on_task_stack_test();
    }

    #[cfg(feature = "m35c2f-ci-trap-frame-test")]
    {
        rt::scheduler::ci_timer_trap_frame_test();
    }

    #[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
    {
        rt::scheduler::ci_resume_interrupt_frame_test();
    }

    #[cfg(feature = "m35c2i-ci-preempt-test")]
    {
        rt::scheduler::ci_timer_round_robin_test();
    }

    #[cfg(feature = "m35c2j-ci-run-queue-test")]
    {
        rt::scheduler::ci_run_queue_test();
    }

    #[cfg(feature = "m35c-ci-guard-fault-test")]
    {
        object::task_guard_fault_ci_test();
    }

    #[cfg(feature = "m35b-ci-panic-test")]
    {
        debug::events::record(
            debug::events::DEBUG_RING_SELFTEST,
            debug::events::SUBSYSTEM_DEBUG,
            debug::events::LEVEL_INFO,
            [0x5041_4e49_4354_4553, 0, 0, 0],
        );
        panic!("M3.5-B deliberate panic test");
    }

    #[cfg(feature = "m35b-ci-fatal-test")]
    {
        debug::events::record(
            debug::events::DEBUG_RING_SELFTEST,
            debug::events::SUBSYSTEM_DEBUG,
            debug::events::LEVEL_INFO,
            [0x4641_5441_4c54_4553, 0, 0, 0],
        );

        // SAFETY: This feature exists only in the dedicated fatal CI image.
        // The address is a canonical virtual page outside every FreeWorld
        // bootstrap/test mapping and is intentionally read to prove #PF dump.
        unsafe {
            let _ = core::ptr::read_volatile(
                0x0000_3000_0000_0000 as *const u64,
            );
        }

        panic!("M3.5-B fatal test unexpectedly returned");
    }

    vfs::init();

    #[cfg(feature = "m4a-ci-self-test")]
    {
        debug::selftest::run_result(
            "m4a.vfs_graph",
            vfs::graph::ci_self_test,
        );
    }

    state::init();
    exec::init();
    rt::init();

    debug::events::record(
        debug::events::BOOT_COMPLETE,
        debug::events::SUBSYSTEM_BOOT,
        debug::events::LEVEL_INFO,
        [0, 0, 0, 0],
    );
    arch::serial::println("FreeWorldOS: M2 APIC timer delivery online; IF enabled");
    arch::serial::println("FreeWorldOS: M1 foundation online");
    arch::serial::println("FreeWorldOS: bootstrap initialization complete");

    #[cfg(feature = "m4b-ci-self-test")]
    {
        vfs::install_ci_scheduler_hook();
    }

    rt::scheduler::start_default_scheduler()
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    panic!("kernel heap allocation failed: {layout:?}");
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    debug::panic::panic(info)
}

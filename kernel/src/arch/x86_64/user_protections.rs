//! CPUID-gated, CPU-local SMEP/SMAP. Activated in every build after
//! bootstrap probes and before the default scheduler starts.
use core::arch::x86_64::__cpuid_count;
use x86_64::registers::control::{Cr4, Cr4Flags};

const SMEP: u32 = 1 << 7;
const SMAP: u32 = 1 << 20;

pub fn enable_supported() {
    x86_64::instructions::interrupts::without_interrupts(|| {
        let cpuid = unsafe { __cpuid_count(7, 0) };
        let smep = cpuid.ebx & SMEP != 0;
        let smap = cpuid.ebx & SMAP != 0;
        let mut flags = Cr4::read();
        if smep { flags |= Cr4Flags::SUPERVISOR_MODE_EXECUTION_PROTECTION; }
        if smap { flags |= Cr4Flags::SUPERVISOR_MODE_ACCESS_PREVENTION; }
        // SAFETY: each CR4 capability was checked with CPUID on this CPU.
        unsafe { Cr4::write(flags); }
        let actual = Cr4::read();
        assert_eq!(actual.contains(Cr4Flags::SUPERVISOR_MODE_EXECUTION_PROTECTION), smep);
        assert_eq!(actual.contains(Cr4Flags::SUPERVISOR_MODE_ACCESS_PREVENTION), smap);
        super::serial::write_fmt(format_args!(
            "FreeWorldOS: M5-W protections: smep={} smap={} cpuid=checked\n",
            u8::from(smep), u8::from(smap)
        ));
        #[cfg(feature = "m5w-smap-fault-test")]
        {
            assert!(smep && smap);
            ci_trigger_smap_fault();
        }
        #[cfg(feature = "m5w-smep-fault-test")]
        {
            assert!(smep && smap);
            ci_trigger_smep_fault();
        }
    });
}

#[cfg(feature = "m5w-smap-fault-test")]
fn ci_trigger_smap_fault() -> ! {
    use crate::memory::{self, PagePermissions};
    const USER: u64 = 0x0000_6000_1000_0000;
    let frame = memory::allocate_frame().expect("M5-W missing test frame");
    unsafe { memory::map_page(USER, frame, PagePermissions::user_read_write()) }
        .expect("M5-W failed to map user page");
    // Positive control: with AC=1, a supervisor read of the *same* mapping
    // must succeed. Stage the byte first via supervisor physical direct map.
    unsafe {
        core::arch::asm!("stac", options(nomem, nostack, preserves_flags));
        core::ptr::write_volatile(USER as *mut u8, 0x5a);
        let value = core::ptr::read_volatile(USER as *const u8);
        core::arch::asm!("clac", options(nomem, nostack, preserves_flags));
        assert_eq!(value, 0x5a);
    }
    super::serial::println("FreeWorldOS: M5-W SMAP positive control: ac=1 read=ok value=0x5a");
    super::serial::println("FreeWorldOS: M5-W SMAP negative: ac=0 expect_pf=0x1 address=0x600010000000");
    let _ = unsafe { core::ptr::read_volatile(USER as *const u8) };
    panic!("M5-W SMAP bypass: forbidden supervisor read returned");
}

#[cfg(feature = "m5w-smep-fault-test")]
fn ci_trigger_smep_fault() -> ! {
    use crate::memory::{self, PagePermissions};
    const USER: u64 = 0x0000_6000_1000_0000;
    let frame = memory::allocate_frame().expect("M5-W missing executable frame");
    unsafe { memory::map_page(USER, frame, PagePermissions::user_read_execute()) }
        .expect("M5-W failed to map executable user page");
    // Return instruction staged in the user executable page through the
    // always-supervisor physical direct map; avoid writing through U/S alias.
    // Use a staging RW map only if mapping permissions are restored RX before
    // the negative execution test.
    let staged = memory::unmap_page(USER).expect("M5-W failed to unmap staging page");
    assert_eq!(staged, frame);
    unsafe { memory::map_page(USER, frame, PagePermissions::user_read_write()) }
        .expect("M5-W failed to map writable staging page");
    unsafe {
        core::arch::asm!("stac", options(nomem, nostack, preserves_flags));
        core::ptr::write_volatile(USER as *mut u8, 0xc3);
        core::arch::asm!("clac", options(nomem, nostack, preserves_flags));
    }
    let staged = memory::unmap_page(USER).expect("M5-W failed to unmap staged code");
    assert_eq!(staged, frame);
    unsafe { memory::map_page(USER, frame, PagePermissions::user_read_execute()) }
        .expect("M5-W failed to map final user executable page");
    super::serial::println("FreeWorldOS: M5-W SMEP negative: cpl=0 target=user_rx expect_pf=0x11 address=0x600010000000");
    // SAFETY: intentionally executes a user RX page at CPL0, in a dedicated
    // fatal-image test. SMEP must fault before executing RET.
    let f: extern "C" fn() = unsafe { core::mem::transmute(USER as usize) };
    f();
    panic!("M5-W SMEP bypass: forbidden supervisor execution returned");
}

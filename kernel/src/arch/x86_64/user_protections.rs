//! M5-W: CPUID-gated boot CPU SMEP/SMAP hardening, enabled only after
//! all legacy self-tests that directly dereference user virtual addresses.
use core::arch::x86_64::__cpuid_count;
use x86_64::registers::control::{Cr4, Cr4Flags};

pub fn enable_and_assert() {
    // Other bootstrap tests can leave IF set. Quiesce delivery during the
    // CPU-local CR4 transition and restore the original IF on return.
    x86_64::instructions::interrupts::without_interrupts(|| {
        let leaf = unsafe { __cpuid_count(7, 0) };
        assert_ne!(leaf.ebx & (1 << 7), 0, "M5-W host lacks SMEP");
        assert_ne!(leaf.ebx & (1 << 20), 0, "M5-W host lacks SMAP");
        let before = Cr4::read();
        unsafe {
            Cr4::write(before | Cr4Flags::SUPERVISOR_MODE_EXECUTION_PROTECTION
                | Cr4Flags::SUPERVISOR_MODE_ACCESS_PREVENTION);
        }
        let after = Cr4::read();
        assert!(after.contains(Cr4Flags::SUPERVISOR_MODE_EXECUTION_PROTECTION));
        assert!(after.contains(Cr4Flags::SUPERVISOR_MODE_ACCESS_PREVENTION));
        super::serial::println("FreeWorldOS: M5-W protection flags: smep=1 smap=1 cpuid=verified");
        #[cfg(feature = "m5w-smap-fault-test")]
        ci_trigger_smap_fault();
    });
}

#[cfg(feature = "m5w-smap-fault-test")]
fn ci_trigger_smap_fault() -> ! {
    use crate::memory::{self, PagePermissions};
    const USER: u64 = 0x0000_6000_1000_0000;
    // Map an actually present user-accessible leaf before attempting to
    // dereference it from CPL0. If this faults, the cause is SMAP (not P=0).
    let frame = memory::allocate_frame().expect("M5-W no test frame");
    unsafe { memory::map_page(USER, frame, PagePermissions::user_read_write()) }
        .expect("M5-W failed to map test user page");
    assert!(Cr4::read().contains(Cr4Flags::SUPERVISOR_MODE_ACCESS_PREVENTION));
    super::serial::println("FreeWorldOS: M5-W deliberate supervisor user virtual read: expect_smap_fault=yes address=0x600010000000");
    // SAFETY: this is a dedicated fatal-test QEMU image. The expected
    // outcome is #PF(P=1,U/S=0), handled by the normal fatal diagnostic path.
    // CLAC makes the test independent of inherited AC state.
    unsafe { core::arch::asm!("clac", options(nomem, nostack, preserves_flags)); }
    let _ = unsafe { core::ptr::read_volatile(USER as *const u8) };
    panic!("M5-W SMAP missing: supervisor user-address read unexpectedly succeeded");
}

//! M5-W: CPUID-gated boot CPU SMEP/SMAP hardening, enabled only after
//! all legacy self-tests that directly dereference user virtual addresses.
use core::arch::x86_64::__cpuid_count;
use x86_64::registers::control::{Cr4, Cr4Flags};

pub fn enable_and_assert() {
    assert!(!super::interrupts_enabled(), "M5-W requires IF clear");
    let leaf = unsafe { __cpuid_count(7, 0) };
    assert_ne!(leaf.ebx & (1 << 7), 0, "M5-W host lacks SMEP");
    assert_ne!(leaf.ebx & (1 << 20), 0, "M5-W host lacks SMAP");
    let before = Cr4::read();
    // SAFETY: both CPUID capabilities were proved, and all intervening
    // kernel bootstrap tests requiring supervisor user-alias access are done.
    unsafe { Cr4::write(before | Cr4Flags::SUPERVISOR_MODE_EXECUTION_PROTECTION
        | Cr4Flags::SUPERVISOR_MODE_ACCESS_PREVENTION); }
    let after = Cr4::read();
    assert!(after.contains(Cr4Flags::SUPERVISOR_MODE_EXECUTION_PROTECTION));
    assert!(after.contains(Cr4Flags::SUPERVISOR_MODE_ACCESS_PREVENTION));
    super::serial::println("FreeWorldOS: M5-W protection flags: smep=1 smap=1 cpuid=verified");
}

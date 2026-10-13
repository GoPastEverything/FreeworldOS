# M5-X — Canonical SYSRET guard

The syscall exit validates the saved return RIP in RCX **on the kernel stack** before any SYSRET. For 48-bit canonical mode, sign extension from bit 47 must equal the original 64-bit value. Canonical returns retain the existing SYSRET behavior. Noncanonical returns must never enter SYSRET.

The controlled fallback builds a ring-3 IRETQ frame for a canonical, unmapped trap address (0x500030003000); the invalid original address is never used as an instruction pointer. The dedicated feature `m5x-return-fault-test` corrupts the saved RCX with 0x0000800000000000. The expected #PF must arrive with CPL3, instruction-fetch error code 0x14, and fallback marker set. This specifically tests corrupting the kernel-held return value, not merely a userspace top-page mapping.

**Limits:** one process, fixed test entry, no scheduler or timer choice. This is a safe controlled abort path, not a generalized signal/exception delivery mechanism. Five-level paging and dynamic return policy are outside scope; revisit canonical width when LA57 support is added.

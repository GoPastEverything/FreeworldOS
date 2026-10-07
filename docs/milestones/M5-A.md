# M5-A — Higher-half kernel / lower-half user split

Status: implementation branch after frozen M4-A through M4-E.

M5-A establishes the address-space ownership rule required before any ring-3 execution exists.

It does **not** add user mode, a process page table, a syscall instruction path, or the native callgate.

## Canonical split

FreeWorld now reserves the x86-64 canonical halves by role:

~~~text
0x0000_0000_0000_1000
    ..
0x0000_7fff_ffff_ffff
        future user mappings only

0xffff_8000_0000_0000
    ..
0xffff_ffff_ffff_ffff
        kernel mappings only
~~~

The null page remains outside the valid user range.

The non-canonical gap between the halves is not a mapping target.

## Fixed bootstrap layout

The bootloader is no longer allowed to choose lower-half dynamic addresses for kernel-owned state.

M5-A configures:

~~~text
ffff_8000_0000_0000  LAPIC mapping (existing)
ffff_8100_0000_0000  kernel image
ffff_8200_0000_0000  bootstrap kernel-stack guard/base
ffff_8300_0000_0000  BootInfo
ffff_9000_0000_0000  FreeWorld heap (existing)
ffff_a000_0000_0000  kernel task stacks (existing)
ffff_c000_0000_0000  physical-memory direct map
ffff_d000_0000_0000  bootloader dynamic range start
ffff_dfff_ffff_f000  bootloader dynamic range end
ffff_e000_0000_0000  kernel mapping self-test region
~~~

The framebuffer and ramdisk mappings remain bootloader-dynamic when present, but the dynamic allocation window is constrained to the higher half.

## Boot-time validation

Before FreeWorld adopts the active page tables, M5-A validates the addresses reported in BootInfo.

The kernel refuses boot if:

- the kernel image is not at the configured higher-half base;
- the kernel image range escapes the higher half;
- the boot stack is not above its configured guard page in the higher half;
- the boot stack range escapes the higher half;
- BootInfo is not at its configured higher-half address;
- the physical-memory direct map is not at its configured higher-half base.

Every successful boot prints the resulting split and concrete bootstrap addresses.

## Generic mapping policy

`memory::map_page()` now enforces address-space ownership in addition to W^X and guard-page reservations.

A mapping with `PagePermissions::user() == false` must target the higher canonical half.

A mapping with `PagePermissions::user() == true` must target:

~~~text
[0x1000, 0x0000_8000_0000_0000)
~~~

Attempts to cross the ownership boundary return explicit errors:

- `KernelMappingOutsideHigherHalf`;
- `UserMappingOutsideLowerHalf`.

This rule applies to the existing heap, LAPIC mapping, task stacks and future generic page mappings.

## Existing self-tests moved

The M1 mapping test and M3.5-A frame-reuse test previously used lower-half supervisor addresses because no user/kernel split existed yet.

M5-A moves those temporary supervisor mappings into the dedicated `ffff_e...` kernel self-test region.

Their ownership/reuse semantics are otherwise unchanged.

## M5-A CI proof

The M5-A self-test uses one allocator-owned frame and proves all four combinations.

First, the two invalid directions are refused without creating a mapping:

~~~text
kernel RW -> lower half  => KernelMappingOutsideHigherHalf
user RW   -> higher half => UserMappingOutsideLowerHalf
~~~

Then the valid directions are exercised:

~~~text
user RW   -> lower half  => map/write/read/unmap succeeds
kernel RW -> higher half => map/write/read/unmap succeeds
~~~

The frame is returned to the recycler only after both temporary mappings have been removed.

Expected marker:

~~~text
FreeWorldOS: M5-A address-space self-test: passed boot_mappings=higher kernel_lower=refused user_higher=refused user_lower=ok kernel_higher=ok null_user=reserved
~~~

Default and combined self-test boots also require the exact fixed bootstrap-layout marker.

## Deferred

M5-A intentionally does not implement:

- user GDT selectors;
- TSS ring-0 privilege stack switching;
- per-process page tables;
- CR3 switching;
- user stacks;
- ring-3 entry;
- FreeWorld ELF startup;
- `FwStartInfoV1`;
- native call-table mapping;
- callgate;
- syscall/trap ABI.

Those begin only after this split is green and frozen.

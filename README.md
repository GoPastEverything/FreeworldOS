# FreeWorldOS

FreeWorldOS is an independent bare-metal Rust operating system aimed at a deterministic, RTOS-influenced execution core with first-class FreeWorld abstractions and compatibility at the edges for PE/Windows and ELF/Linux software.

> **Founding law:** Compatibility at the edges. Originality inside.

## Status

**M1 — CPU exception and memory foundation.** FreeWorldOS now boots with its own x86_64 GDT/TSS/IDT, logs defined CPU exceptions, discovers firmware-reported usable memory, owns the active page-table mutation path, allocates physical frames, maps/unmaps 4 KiB pages, enables NX, and rejects W+X for FreeWorld-created mappings. GitHub CI boots both the default and self-test BIOS kernels under QEMU and proves the memory/exception paths.

Nothing here claims PE/ELF/WinFacet/LinuxFacet compatibility yet. Those are staged milestones and will be implemented only against clean-room, behavior-driven specifications.

## Architecture laws

1. Compatibility at the edges. Originality inside.
2. Foreign compatibility terminates at FreeWorld abstractions.
3. One native object, multiple compatibility projections.
4. Path syntax is presentation; object identity is native.
5. RegCube stores state; the VFS stores files.
6. File format, operating environment, ABI, and CPU architecture are independent properties.
7. x86_64 is the first implementation target; the architecture must not hard-code x86_64 as the universal model.

## Bootstrap target

Initial hardware target: **x86_64 PC, BIOS or UEFI**.

The kernel uses Rust's freestanding `x86_64-unknown-none` target. The current boot-image tooling uses `bootloader`/`bootloader_api` 0.11.17 as a bootstrap dependency; it is not part of FreeWorldOS's long-term architectural identity.

## Build

Requirements:

- `rustup`
- QEMU (`qemu-system-x86_64`)
- the pinned nightly toolchain from `rust-toolchain.toml`

```bash
cargo build
cargo run -- uefi
# or
cargo run -- bios
```

The first successful boot should print a short initialization trace to COM1/QEMU serial output and then halt.

## Repository layout

```text
kernel/src/
  arch/       CPU and platform bring-up
  exec/       execution profiles and later image/facet dispatch
  memory/     architecture-neutral physical/virtual memory interface
  object/     native FW process/module/handle model
  rt/         architecture-neutral runtime time core; scheduler lands after M3
  state/      RegCube
  vfs/        native namespace and path/object projections

docs/
  architecture/
  cleanroom/
  milestones/
```

## Near-term milestones

- **M0:** ✅ boot x86_64 under QEMU; serial output; panic path
- **M1:** ✅ GDT/TSS/IDT, exception handling, hardened bootstrap frame allocation, page map/unmap, NX and W^X on FreeWorld-created mappings
- **M2:** ✅ PIC/APIC bring-up, uncached LAPIC mapping, PIT calibration, first `sti`, periodic tick delivery, and generic `rt::time` hook
- **M3:** ◐ reusable kernel heap + first native object + generational handle table + READ/WRITE rights + close/reclamation proof implemented on branch
- **M3.5:** task stacks + saved context + deterministic scheduler/context switching built on M3 primitives
- **M4:** VFS namespace graph + RAM filesystem + mount/root projections
- **M5:** user mode + syscall/trap ABI for FreeWorld-native processes
- **M6:** ELF image decoder and minimal LinuxFacet syscall interception
- **M7:** PE image decoder and minimal WinFacet DLL/API surface
- **M8:** FW_MODULE loader and Bridge ABI v1
- **M9:** RegCube transactional state core and WinFacet registry projection

See `docs/architecture/CANDIDATE-v0.1.md` for the current design baseline and `docs/milestones/` for the frozen M1/M2 proofs and current M3 ownership proof.

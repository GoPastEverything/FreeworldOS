# FreeWorldOS Native ABI v0.1 — Locked Decisions

Status: locked architectural decisions for the first FreeWorld-native userspace.

This document defines the boundary between FreeWorld-native applications and the FreeWorldOS kernel. It does not define LinuxFacet or WinFacet behavior.

## FW-NATIVE-01 — Native executable container

FreeWorld-native executables use ELF64 as the container format.

Format and execution environment are independent: an ELF file is not automatically a Linux process.

Until FreeWorldOS has an officially assigned ELF OSABI value, EI_OSABI remains NONE/generic. FreeWorldOS does not squat on an unassigned global OSABI number.

A native FreeWorld process is selected only by the explicit FreeWorldOS PT_NOTE defined by FW-NOTE-01.

Classification rules:

- a valid FreeWorld native note with environment NATIVE selects the FreeWorld execution environment;
- an ELF file without that note may be eligible for LinuxFacet classification;
- contradictory or malformed metadata is a load error;
- the loader does not guess.

For the M5 native loader, PT_INTERP is not supported. A FreeWorld-native ELF carrying PT_INTERP is rejected. A later ABI revision may define a FreeWorld dynamic-interpreter contract explicitly.

## FW-NOTE-01 — Exact FreeWorld ELF note

The FreeWorld ABI marker must be reachable through an ELF PT_NOTE program header. Section headers are not required and must not be relied on.

The v1 note uses normal ELF note framing:

- namesz: 12
- descsz: at least 16 for ABI v1
- type: 1
- name bytes: "FreeWorldOS\0"
- name and descriptor are padded according to ELF note alignment rules

The first 16 descriptor bytes are four little-endian u32 values:

    offset  size  field
    0x00    4     abi_version
    0x04    4     environment
    0x08    4     flags
    0x0c    4     reserved

Version 1 values:

- abi_version = 1
- environment = 1 means native FreeWorld
- flags = 0
- reserved = 0

Rules:

- unknown abi_version: reject before creating the process;
- unknown environment: reject;
- nonzero reserved in v1: reject;
- unknown required flags: reject;
- a descriptor larger than the minimum v1 size is permitted when abi_version is understood; a v1 loader ignores an unrecognized appended tail;
- a descriptor smaller than the minimum for its version is malformed and rejected.

The ABI note is a process-classification contract, not a promise that ELF internals become FreeWorldOS internals.

## FW-ABI-01 — ABI source of truth

The repository will eventually contain:

- crates/fw-abi
- crates/fw-sys
- crates/libfw
- crates/fwlibc
- user/

fw-abi is a tiny no_std crate shared by the kernel and native userspace. It defines deliberately ABI-stable types and versioned contracts such as:

- handles
- rights
- object identifiers
- status/error codes
- startup records
- call-table layouts
- public function signatures
- versioned public structures

Public structures are explicitly ABI-safe, for example repr(C), and listed in an ABI manifest. Ordinary Rust struct layout is never treated as public ABI merely because it compiles.

Generated C headers may be produced from the deliberate ABI definitions, but code generation does not decide what is ABI.

## FW-START-01 — Native process startup and callgate discovery

M5 must support statically linked native binaries without requiring a dynamic linker and without reserving a dedicated general-purpose register.

The kernel therefore publishes callgate discovery data on the initial userspace stack.

On x86_64 v1, the native entry stack is:

    RSP -> argc: u64
           argv[0..argc]: u64 pointers
           0: u64
           envp[0..n]: u64 pointers
           0: u64
           padding as needed for 8-byte alignment
           FwStartInfoV1

FwStartInfoV1 is an ABI structure:

    struct_size:    u32
    abi_version:    u32
    callgate_base:  u64
    callgate_size:  u64
    call_table:     u64
    reserved0:      u64

Requirements:

- struct_size allows compatible extension by appending fields;
- abi_version must be supported by the startup stub;
- callgate_base/callgate_size identify the executable user mapping that is permitted to issue native kernel-entry instructions;
- call_table points to FwCallTableV1;
- reserved0 must be zero in v1.

FwCallTableV1 begins with:

    version:    u32
    size_bytes: u32
    fn[...]:    u64 function pointers in the v1 ABI slot order

The concrete slot order belongs to fw-abi and becomes append-only within ABI v1 once published.

Startup behavior:

1. the fw-sys startup stub locates FwStartInfoV1 after envp;
2. it validates struct_size and abi_version;
3. it validates the call table version and size;
4. it stores the table pointer in process-local runtime state;
5. it refuses to enter main if a required v1 slot is missing.

A v1 binary ignores extra table slots it does not understand.

The discovery record is not a syscall mechanism. It only tells userspace where the kernel-supplied callgate and function table are.

A future shared-object callgate may export stable symbol names. That may replace table-based discovery internally without changing the higher libfw API.

## FW-SYSCALL-01 — Kernel-mapped callgate owns native kernel entry

Native application code does not issue the architecture syscall instruction directly.

M5 runtime call path:

    application
        |
        v
    libfw / fwlibc
        |
        v
    fw-sys
        |
        v
    versioned call table
        |
        v
    kernel-mapped FreeWorld callgate
        |
        v
    private kernel-entry protocol
        |
        v
    kernel

Only the kernel-supplied callgate mapping contains the architecture syscall instruction.

fw-sys contains unsafe wrappers that dispatch through the validated call table. It is not the gate and must not embed raw native syscall numbers or the syscall instruction.

This preserves a public userspace ABI while allowing the private callgate-to-kernel protocol to be renumbered or redesigned.

Security direction:

- the callgate mapping is executable and not writable by the process;
- the kernel may require that the saved userspace instruction pointer at native syscall entry lies within the process's approved callgate mapping;
- this enforcement is a security property, not the discovery mechanism.

LinuxFacet is separate: Linux programs may issue Linux syscall numbers directly and LinuxFacet translates them.

WinFacet is separate: PE programs call their expected DLL interfaces and WinFacet translates them.

fwlibc is not glibc. WinFacet system DLLs are not fwlibc.

## Public ABI evolution

Public ABI does not permit silent signature changes.

Within call-table ABI v1:

- published slots are append-only;
- an existing slot's function signature does not change;
- new optional functions append new slots;
- a binary checks size before using a slot.

Broader ABI evolution uses one or more of:

- a new ABI version;
- new functions rather than mutating old signatures;
- versioned structures containing size/version fields.

Only the private callgate-to-kernel protocol is freely replaceable.

## FW-ERR-01 — Native status model

The native error type is:

    fw_status_t = i32

Meaning:

- 0 = success;
- negative values = errors;
- positive values are reserved for future use.

There is no kernel-owned per-thread errno and no errno state inside fw-sys.

Native layers map status according to language/runtime needs:

- libfw converts fw_status_t into Rust Result/error types;
- fwlibc converts relevant failures into C errno values using TLS owned by fwlibc;
- LinuxFacet preserves Linux-visible error behavior independently;
- WinFacet preserves Windows-visible error behavior independently.

C errno therefore does not shape the native FreeWorld kernel ABI.

The concrete status-code registry will live in fw-abi before M5 publishes the first native userspace ABI.

## FW-CAP-01 — Handle/capability model

Operations on kernel objects are handle-based and capability/rights checked.

Examples:

- object/file/channel/process operations consume handles with required rights;
- channel creation returns handles;
- non-object operations such as thread yield or monotonic clock reads do not acquire fake handles merely for stylistic uniformity.

The exact rights set remains a later object-model decision.

## FW-LIBC-01 — FreeWorld-owned C library

fwlibc is a FreeWorld-owned C runtime/standard library implementation, primarily written in Rust and layered over fw-sys.

Initial order:

1. freestanding C essentials;
2. allocation, strings, memory and formatting;
3. native files/process/time APIs exposed through C;
4. threads and synchronization;
5. a selected POSIX convenience subset.

POSIX compatibility is not the FreeWorld kernel model.

fwlibc is not intended to replace glibc or musl for LinuxFacet applications.

## FW-RUST-01 — Rust userspace sequence

Early native Rust programs use no_std, then alloc when available, plus libfw.

Full Rust std support and a dedicated target such as x86_64-unknown-freeworld come only after the native ABI has stabilized enough to avoid designing the kernel around current Rust std internals.

The M5 proof is intentionally small:

    FreeWorld ELF
        -> native loader
        -> ring 3
        -> libfw
        -> fw-sys
        -> call table
        -> callgate
        -> kernel
        -> one successful userspace-visible action

The first proof target is a no_std native program that emits one line through the FreeWorld API.

## FW-KERNEL-01 — Kernel/service boundary direction

FreeWorldOS uses a mechanism-focused privileged core with service-oriented higher layers.

Privileged core candidates:

- interrupts/exceptions
- scheduler
- virtual memory
- address spaces
- threads/process primitives
- handle/capability tables
- IPC primitive
- timers
- syscall/callgate dispatch

Service-boundary candidates:

- RegCube
- filesystem services
- compatibility facets
- graphics
- higher-level device management

This is not a declaration that FreeWorldOS is a strict microkernel.

Driver placement remains empirical. Userspace/service drivers are a goal where practical, not an M1 constraint.

## Repository layout decision

The intended userspace layout is:

    crates/
        fw-abi/
        fw-sys/
        libfw/
        fwlibc/
    user/

No additional crates are introduced until a real boundary justifies them.

These crates do not need to be created before the M5 work actually consumes them.

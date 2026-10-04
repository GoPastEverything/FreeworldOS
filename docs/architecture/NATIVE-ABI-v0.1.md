# FreeWorldOS Native ABI v0.1 — Locked Decisions

Status: locked architectural decisions for the first FreeWorld-native userspace.

This document defines the boundary between FreeWorld-native applications and the FreeWorldOS kernel. It does not define LinuxFacet or WinFacet behavior.

## FW-NATIVE-01 — Native executable container

FreeWorld-native executables use ELF64 as the container format.

A FreeWorld native binary is identified by an explicit FreeWorldOS ABI note. Until an official ELF OSABI value is assigned, EI_OSABI remains NONE/generic; FreeWorldOS does not squat on an unassigned global OSABI number.

The native note declares at minimum:

- owner: FreeWorldOS
- ABI version: 1
- execution environment: native FreeWorld

Format and environment are independent: an ELF file is not automatically a Linux process.

Classification rule:

- an explicit valid FreeWorld native note selects the FreeWorld execution environment;
- a normal Linux ELF without that note is eligible for LinuxFacet classification;
- contradictory metadata is an error, not something the loader guesses around.

A FreeWorld-native executable must not silently inherit Linux semantics just because it is ELF.

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
- public callgate function signatures
- versioned public structures

Public structures are explicitly ABI-safe, for example repr(C), and listed in an ABI manifest. Ordinary Rust struct layout is never treated as public ABI merely because it compiles.

Generated C headers may be produced from the deliberate ABI definitions, but code generation does not decide what is ABI.

## FW-SYSCALL-01 — Kernel-mapped callgate

Native programs do not issue the architecture syscall instruction directly.

Runtime call path:

    application
        |
        v
    libfw / fwlibc
        |
        v
    fw-sys
        |
        v
    kernel-mapped FreeWorld callgate
        |
        v
    private kernel-entry protocol
        |
        v
    kernel

Only the kernel-supplied callgate contains the architecture syscall instruction.

fw-sys contains unsafe declarations/wrappers around stable callgate symbols. It is not the gate and must not embed raw native syscall numbers or the syscall instruction.

This preserves a stable public userspace ABI while allowing the private callgate-to-kernel protocol to be renumbered or redesigned.

The kernel may later enforce that native entry originates from the mapped callgate region.

LinuxFacet is separate: Linux programs may issue Linux syscall numbers directly and LinuxFacet translates them.

WinFacet is separate: PE programs call their expected DLL interfaces and WinFacet translates them.

fwlibc is not glibc. WinFacet system DLLs are not fwlibc.

## Public ABI evolution

Stable symbol names do not permit silent signature changes.

Public ABI evolves through one or more of:

- new symbols, for example foo2 rather than mutating foo;
- versioned structures containing size/version fields;
- explicit ABI-version transitions.

Only the private callgate-to-kernel protocol is freely replaceable.

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

# FreeWorldOS Design Specification v0.1 (Candidate)

Status: current source of truth for the reconstructed design. It remains candidate until the original FreeWorldOS / Player One Start OS conversation bodies are recovered and compared against it.

This document supersedes the earlier short recovery summary. Recovered original material is never silently overwritten; it is compared section-by-section and merged deliberately.

---

## 1. Founding law

Compatibility at the edges. Originality inside.

FreeWorldOS does not reproduce the internal architecture of Windows or Linux.

For every compatibility feature, ask:

What behavior does the binary require at the interoperability boundary, and what is the best FreeWorldOS-native mechanism underneath it?

Windows and Linux interfaces are compatibility surfaces. FreeWorldOS remains the operating system underneath them.

A second architectural rule follows from this:

One native object, multiple compatibility projections.

---

## 2. Primary objective

FreeWorldOS is an independent bare-metal Rust operating system with an RTOS-influenced execution core.

Initial executable ecosystems:

- FreeWorldOS-native programs
- PE/COFF executables and DLLs
- ELF executables and shared objects

PE and ELF are first-class image formats. Neither format defines the internal process model.

Initial CPU implementation target: x86_64.

### Native foreign-image execution invariant

For a binary whose ISA matches the running machine, PE/COFF and ELF compatibility means direct native execution, not virtualization or CPU emulation.

- the program's compiled instructions execute directly on the processor under FreeWorldOS scheduling;
- FreeWorldOS maps/relocates the PE or ELF image and resolves its DLL/SO dependencies;
- Windows/Linux observable OS behavior is supplied by WinFacet/LinuxFacet and FreeWorld-native objects;
- application-provided matching-ISA DLLs and shared objects execute as native mapped modules;
- compatibility does not require a guest Windows or Linux kernel.

System-facing compatibility libraries/interfaces are FreeWorldOS clean-room implementations. Proprietary Windows implementation code and copied glibc implementation code are not part of the design.

The architecture is ISA-neutral enough to add ARM64 later. Cross-ISA execution is a separate translation problem and is not part of v0.1. A foreign-ISA binary cannot satisfy the direct-native invariant on a mismatched CPU without a separately designed translation layer.

---

## 3. Execution profile

Every process is a FreeWorld process.

FreeWorldOS keeps four properties independent:

~~~text
environment     image format     ABI                architecture
-----------     ------------     ---                ------------
FreeWorld       FreeWorld        FreeWorld64        x86_64
Windows         PE               Microsoft x64      x86_64
Linux           ELF              System V x64       x86_64

future examples:
Windows         PE               Windows ARM64      ARM64
Linux           ELF              AAPCS64            ARM64
~~~

Conceptually:

~~~text
ExecutionProfile {
    environment,
    image_format,
    abi,
    architecture
}
~~~

File format != operating environment != ABI != CPU architecture.

There is no fundamental WindowsProcess or LinuxProcess.

---

## 4. Universal image loading

The image loader identifies an executable and invokes the appropriate decoder.

~~~text
PE  ----\
        \
ELF -----+----> FW image/process/module objects
        /
FW  ----/
~~~

PE and ELF parsing remain distinct where the formats require it, but both populate FreeWorld-native runtime objects.

---

## 5. FW_PROCESS and FW_THREAD

Processes and threads are native FreeWorld objects.

A process owns or references FreeWorld concepts such as:

- address space
- handle/capability space
- module space
- security context
- execution profile
- namespace projection

Compatibility behavior is attached to the process through its execution profile and facet dispatch. It does not replace the FreeWorld process model.

---

## 6. FW_MODULE

DLL and SO loading converge on one format-neutral internal module object.

~~~text
PE DLL -> PE decoder ----\
                         +--> FW_MODULE
ELF SO -> ELF decoder ---/
~~~

FW_MODULE is expected to represent, as needed:

- mapped image regions
- symbols
- imports and exports
- relocations
- constructors/destructors
- TLS metadata
- unwind metadata
- ABI metadata
- ownership/security identity

PE-specific constructs such as import tables, ordinals and DllMain, and ELF-specific constructs such as DT_NEEDED, PLT/GOT, symbol versions and init_array, are decoded into the FreeWorld model without pretending the two formats are identical.

---

## 7. WinFacet

WinFacet presents the observable behavior required by compatible PE software and translates toward FreeWorld primitives.

~~~text
CreateFileW()
    |
    v
WinFacet
    |
    v
fw_object_open()
    |
    v
FreeWorld VFS
~~~

Expected work includes:

- PE image loading
- DLL discovery/loading
- imports/exports and relocations
- Microsoft x64 ABI behavior
- TLS
- SEH/unwind metadata
- required Win32 interfaces
- NT-compatible interfaces where interoperability requires them
- filesystem/process/thread/synchronization behavior
- PEB/TEB-visible layouts only where programs directly depend on them
- COM and graphics compatibility in later milestones

Required external names may match what binaries import. Their implementations remain FreeWorldOS code.

For Windows compatibility, the practical stable boundary is the documented user-facing system-DLL/API surface rather than undocumented low-level syscall numbers. FreeWorldOS therefore supplies clean-room compatibility implementations for system-facing interfaces such as `ntdll`/`kernel32`-class APIs as required, while application-provided matching-ISA DLLs load and execute as shipped.

---

## 8. LinuxFacet

Linux compatibility is layered:

1. ELF executable format
2. architecture ABI, initially System V x86_64
3. Linux userland ABI
4. libc expectations such as glibc and musl
5. Linux UAPI / syscall compatibility

Many dynamically linked programs reach the kernel through libc, while static programs and some runtimes issue Linux syscalls directly. Therefore Linux syscall compatibility is required independently of libc strategy.

For Linux compatibility, the stable kernel boundary is the Linux syscall/UAPI contract. A matching-ISA distro-provided glibc or musl shared object is expected to load and execute natively like any other ELF `.so`; FreeWorldOS does not need to replace that application/userland libc merely to provide Linux compatibility. LinuxFacet implements the required syscall-facing semantics underneath it.

Frozen requirement:

FreeWorldOS provides a native, low-overhead Linux syscall interception path.

A LinuxFacet thread's syscall transition enters the FreeWorld trap path, which dispatches according to the execution profile and translates Linux semantics into FreeWorld operations.

Still open: whether the bulk of Linux syscall semantics runs in kernel space, a privileged FreeWorld subsystem, or a hybrid. Only the low-overhead trap and dispatch requirement is frozen.

---

## 9. FreeWorld native API and callgate

FreeWorldOS defines its own first-class API.

Candidate families include:

~~~text
fw_object_*
fw_process_*
fw_thread_*
fw_vm_*
fw_channel_*
fw_module_*
fw_cube_*
~~~

Compatibility facets translate toward these primitives. Neither Win32/NT nor Linux UAPI becomes the FreeWorld native API.

Native userspace reaches the kernel through a kernel-supplied callgate rather than embedding the private kernel-entry protocol in applications or libraries.

Runtime call path:

~~~text
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
~~~

Only the callgate issues the architecture syscall instruction. fw-sys dispatches through a versioned function-pointer table discovered from the initial native process stack; it does not contain raw native syscall numbers or the syscall instruction.

For M5, the kernel places a versioned startup record after argc/argv/envp. That record identifies the callgate mapping and points to the append-only v1 call table. This supports statically linked native binaries without requiring a dynamic linker or reserving a dedicated general-purpose register.

FreeWorld-native ELF files are marked by a precise PT_NOTE contract and rejected before process creation when they request an unsupported native ABI version. Native errors use fw_status_t (i32): zero success, negative error, positive reserved. libfw maps statuses to Rust results; fwlibc owns any C errno TLS mapping.

The public native ABI is versioned/stable. The private callgate-to-kernel protocol may change without recompiling all native applications.

See NATIVE-ABI-v0.1.md for the exact note, startup, call-table, syscall and error contracts.

---

## 10. Bridge ABI

FreeWorldOS may support deliberate calls between modules using different ABIs through a third, FreeWorld-defined Bridge ABI.

Version 1 is intentionally narrow and C-like.

Permitted candidates:

- fixed-width integers
- floating-point primitives
- byte buffers
- strings with explicit encoding
- immutable structures declared in a Bridge IDL
- FreeWorld handles
- explicitly declared callbacks

Not promised in v1:

- arbitrary C++ object layouts
- RTTI compatibility
- cross-ABI exception propagation
- SEH to DWARF exception conversion
- allocator ownership transfer
- unrestricted variadic calls

The first bridge should be explicit and reliable rather than clever.

---

## 11. RegCube native state fabric

RegCube stores state. VFS stores files.

RegCube is not a reimplementation of the Windows Registry. Registry behavior is one compatibility projection onto a FreeWorld-native state system.

A RegCube cell's hot-path address has four fields:

~~~text
namespace + identity + scope + property
~~~

Example conceptual address:

~~~text
cube://application/example/user/ui.theme
~~~

Interpretation:

~~~text
namespace = application
identity  = example
scope     = user
property  = ui.theme
~~~

Revision is history, not part of the lookup key.

Schema is write-time metadata, not part of the ordinary read key.

A common read should resolve the cell address directly to its current revision without walking history or re-validating its schema.

Candidate RegCube capabilities:

- atomic transactions
- revision history
- rollback by creating a new current revision from an older value
- MVCC
- per-identity isolation
- capability-mediated cross-identity access
- machine/user/session/application scopes
- optional schemas
- watchers/subscriptions
- overlays
- provenance
- optional synchronization

---

## 12. Windows RegCube projection

Windows registry calls can be projected through WinFacet onto RegCube.

~~~text
RegQueryValueEx
      |
      v
WinFacet registry projection
      |
      v
RegCube cell
~~~

The observable registry behavior must match what compatible software expects. The underlying storage does not need registry hives or Windows's internal registry architecture.

---

## 13. Linux configuration and RegCube boundary

Linux configuration files remain ordinary VFS files by default, including files under paths such as ~/.config.

RegCube never silently absorbs arbitrary filesystem data.

A compatibility facet may expose an explicit synthetic file backed by FreeWorld state when required, for example a generated system configuration view.

Examples:

~~~text
/etc/resolv.conf -> synthetic LinuxFacet view of FreeWorld network state
/proc/*           -> synthetic process/system providers
/sys/*            -> synthetic device/system providers
~~~

The rule is:

State belongs in RegCube. Files belong in VFS. Projections are explicit.

---

## 14. FreeWorld VFS and namespace model

FreeWorldOS has one native mount/object graph containing root/volume objects.

A path is not fundamentally a Windows or Linux string. Internally it is:

~~~text
root object + sequence of native name segments
~~~

Each process receives a namespace projection.

Examples:

~~~text
Native FreeWorld           WinFacet                 LinuxFacet
----------------           --------                 ----------
<C:>Games\Game.exe         C:\Games\Game.exe       /volumes/windows-c/Games/Game.exe
</>sbin/ifconfig           projected root/drive     /sbin/ifconfig
~~~

Cross-facet visibility is policy-driven and explicit, not automatic.

### 14.1 Native names are binary-safe

FreeWorldOS must not model filenames as Rust str.

Linux names are arbitrary byte sequences except NUL and slash.

Windows stores names as UTF-16 and real names can contain unpaired surrogate code units that do not have a normal UTF-8 representation.

Therefore the native VFS name type is:

~~~text
raw bytes + encoding/provenance tag
~~~

Initial encodings:

- opaque bytes for Linux-originated names
- WTF-8 for lossless Windows UTF-16 round-tripping, including unpaired surrogates

The encoding tag is not a claim that the bytes are well-formed human text.

Path separators are projection syntax and are never stored inside a native name segment.

### 14.2 Name and case policy

Directories may carry name-comparison policy such as:

- case-sensitive
- case-insensitive
- case-preserving, case-insensitive

A source-code tree and a Windows-oriented game directory can therefore coexist under different rules.

### 14.3 Foreign naming conflicts

FreeWorldOS should store names its native object model can represent.

If a name cannot be expressed directly through a facet's normal syntax, the facet must use a reversible projection or escape rather than losing object identity.

---

## 15. On-disk filesystem drivers

Namespace syntax and disk format are separate problems.

FreeWorldOS may eventually read and write multiple disk formats natively, including families such as:

- FAT and exFAT
- NTFS
- ext4
- XFS
- btrfs

An ELF process can access an object physically stored on an NTFS volume, and a PE process can access an object physically stored on ext4, subject to namespace and security policy.

The native VFS/object model must be expressive enough to project differing foreign semantics such as:

- ACLs versus UID/GID/mode bits
- Windows share/delete semantics versus Unix unlink lifetime
- advisory and mandatory locking
- symlinks, junctions and hard links
- alternate data streams and extended attributes
- timestamps and file attributes

---

## 16. RTOS direction

This is a directional kernel-shape decision, not a frozen scheduler algorithm.

FreeWorldOS is being built as a bare-metal Rust, RTOS-influenced system with:

- deterministic scheduling as a design goal
- explicit task states
- bounded kernel mechanisms where practical
- capability-oriented object access
- clear mechanism/policy separation
- compatibility layers outside the identity of the core kernel

Still intentionally open until timing and interrupt infrastructure can be measured:

- scheduler class
- preemption policy
- SMP scheduling
- priority inheritance details
- hard versus soft real-time guarantees
- deadline semantics

The project must not claim real-time guarantees before they are measured.

---

## 17. Clean-room engineering

Compatibility work keeps written provenance.

Repository structure:

~~~text
docs/cleanroom/
    SOURCES.md
    INTEROP_REQUIREMENTS.md
    API_PROVENANCE.md
    BEHAVIOR_TESTS.md
~~~

Each compatibility behavior records:

- feature
- required observable behavior
- public specification/source
- independent behavioral test
- FreeWorld implementation path
- interoperability-required layouts/constants, if any
- copied proprietary source: NONE

No proprietary, leaked, copied, or disassembly-derived implementation code is used.

---

## 18. Compatibility testing

Compatibility testing is behavior-driven.

Where legally and technically appropriate:

~~~text
reference environment -> observable result
                           ||
FreeWorldOS ----------> observable result
~~~

Requirements are derived from public specifications and lawfully observable behavior, not copied implementation internals.

---

## 19. Non-goals

FreeWorldOS is not:

- a Linux distribution
- Linux with Wine bundled in
- a Windows clone
- a Windows theme over Linux
- a VM presented as compatibility
- two operating systems glued together
- an internal NT reimplementation

PE and ELF compatibility are capabilities of FreeWorldOS, not its identity.

---

## 20. Bootstrap and milestone sequence

Current sequence:

- M0: bare-metal x86_64 boot, serial output, panic/halt path, BIOS+UEFI image generation, CI build and boot proof
- M1: GDT/IDT, exception handling, physical/virtual memory discovery
- M2: timer/APIC bring-up and measurable deterministic scheduler foundation
- M3: heap, FW object/handle core, capability skeleton
- M4: VFS mount/object graph, binary-safe names, RAM filesystem, namespace projections
- M5: native ELF PT_NOTE validation, ring 3, versioned initial-stack startup record, kernel-mapped callgate and call table, fw-abi/fw-sys/libfw minimum, status-code ABI, and a no_std FreeWorld userspace hello

### M5 address-space prerequisite

Before FreeWorld creates ring-3 process page tables, the x86_64 address-space split must be explicit:

- the lower canonical half belongs to process/user mappings, except deliberately defined shared user mappings such as the native callgate;
- kernel-private mappings live in the higher half and remain present consistently across process address spaces;
- kernel-private mappings include the kernel image, privileged physical-memory/direct map, kernel heap, task stacks, and device mappings;
- the current bootstrap placement of the kernel image and physical-memory map must therefore be relocated/hardened before M5 user address spaces are considered complete;
- once SMAP is enabled, every kernel task switch/trap-return path must ensure the AC flag is cleared unless an explicit, bounded user-access section is active; AC must never leak from one task to another.

M3.5-C task stacks are moved into the higher half before their first saved context is created so no scheduler state depends on a lower-half kernel-stack address.
- M6: ELF decoder and initial LinuxFacet syscall interception
- M7: PE decoder and initial WinFacet DLL/API surface
- M8: FW_MODULE loader and Bridge ABI v1
- M9: RegCube transaction core and registry projection

---

## 21. Reproducible bootstrap inputs

Initial implementation pins:

- Rust nightly toolchain in rust-toolchain.toml
- rust-osdev bootloader generation in Cargo.toml
- ovmf-prebuilt crate version
- explicit OVMF/EDK2 release rather than Source::LATEST

These are bootstrap dependencies, not FreeWorldOS architectural dependencies.

---

## 22. Unresolved design questions

Intentionally open:

1. final kernel architecture terminology: monolithic, microkernel, hybrid, or a FreeWorld-specific description
2. exact FW_OBJECT semantics
3. native IPC model
4. native driver architecture
5. native graphics/windowing architecture
6. Bridge ABI IDL
7. RegCube physical storage engine
8. WinFacet compatibility target baseline
9. LinuxFacet UAPI baseline
10. graphics compatibility strategy
11. minimum Win32/NT surface for first PE application milestone
12. ARM64 implementation schedule
13. cross-ISA translation architecture

---

## 23. Recovery rule

This specification is the repository's current design source of truth, but it does not erase original project history.

When original FreeWorldOS / Player One Start OS material is recovered:

1. recover the complete transcript or artifact
2. compare it against this specification section-by-section
3. preserve original intent where it materially differs
4. merge later improvements deliberately
5. revise the specification and bump its version

Nothing recovered is silently overwritten.

Highest-value recovery target: the original RegCube design discussion.

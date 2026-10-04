# FreeWorldOS System Tools v0.1 — Locked Decisions

Status: locked architectural decisions for FreeWorldOS native administration, diagnostics, developer tooling, and system tools.

This document defines the FreeWorld-native tools model and the structured administration protocol those tools use. It does not define Linux or Windows command compatibility. Linux and Windows programs remain free to use their own native tools inside their own execution environments.

The command-line program `world` is one client of the administration protocol. It is not the protocol itself.

## Founding principles

1. **Native first.** Every FreeWorld tool is a FreeWorld-native program built on `libfw` or `fwlibc`, using the native callgate and FreeWorld object model. FreeWorld system tools are not ports of Windows or Linux utilities.

2. **Objects, not text files.** Tools operate on FreeWorld objects, handles, capabilities, VFS objects, devices, and RegCube cells. Human-readable text is a presentation view, not the machine interface other tools must parse.

3. **Structured output by default.** Tools exchange typed records, raw byte streams, or event streams. Text rendering is a terminal/UI concern.

4. **Capabilities are visible.** Authority is explicit. Tool operations can expose which handles and rights authorized an action. Hidden ambient authority is not the native FreeWorld model.

5. **One tool model, both worlds.** A native FreeWorld file or device tool operates on FreeWorld objects independent of whether the underlying storage, path projection, or application environment is Windows-like or Linux-like.

6. **Original FreeWorld interface.** FreeWorld does not clone Windows or Unix tool suites. Generic concepts and ordinary verbs such as `list`, `show`, `copy`, `move`, `start`, and `stop` are not proprietary ideas and may be used where they make the interface clearer.

7. **Testable in CI.** Kernel support and user-mode tools gain named behavior tests as they land. QEMU remains the baseline CI environment for milestones that can be proved there.

8. **Protocol before presentation.** The FreeWorld administration protocol is the durable system interface. `world`, graphical administration applications, remote management clients, and future automation are clients of that same protocol.

## Administration stream model

FreeWorld administration uses three first-class stream kinds.

### RECORD

A RECORD stream carries typed structured records.

Examples:

- process information;
- object and handle metadata;
- device descriptions;
- filesystem entries;
- rights/authority information;
- RegCube cells;
- network connection records.

Records are not terminal-formatted strings.

### BYTE

A BYTE stream carries arbitrary bytes.

Examples:

- file contents;
- executable images;
- archives;
- images;
- compressed data;
- packet-capture payloads.

Binary data is not forced into a record representation.

### EVENT

An EVENT stream carries structured event records.

Examples:

- kernel events;
- diagnostics;
- tracing;
- tool progress;
- warnings;
- authority receipts;
- audit events.

RECORD, BYTE, and EVENT are distinct stream kinds. Conversions between unlike stream kinds are explicit operations.

For example, rendering records to text or JSON is a conversion from RECORD to BYTE. Parsing a byte format into records is a conversion from BYTE to RECORD.

## `world` command namespace

The canonical native command-line client is:

~~~text
world <area> <verb> [target] [options]
~~~

The area table is locked as follows:

| Area | Covers |
| --- | --- |
| `system` | machine overview, boot, recovery, power |
| `process`, `object` | processes, threads, objects, handles and rights |
| `driver`, `service` | drivers and long-lived services |
| `memory` | frames, heap, address spaces |
| `disk` | block devices, partitions, format and check |
| `fs` | files, mounts, case policy, name escapes, path translation |
| `access` | capabilities, profiles, accounts, audit log |
| `cube` | RegCube |
| `compat` | Windows and Linux compatibility tools |
| `device` | hardware |
| `net` | networking |
| `graphics` | displays and desktop |
| `log`, `debug` | event log and debugging |

Path translation belongs under `world fs`; it is not a separate top-level area.

Example commands:

~~~text
world system show
world process list
world object show <handle>
world object handles
world service start <name>
world driver list
world memory heap
world memory frames
world disk list
world fs copy <source> <destination>
world fs path <path> --view windows
world fs path <path> --view linux
world access show <handle>
world access trace <handle>
world compat explain <binary>
world device list
world net connections
world log show
~~~

The exact shell grammar is defined later under FW-SHELL-01. These examples define intent, not a finalized parser grammar.

## Tool families

Fourteen native tool families cover the planned system.

| Family | `world` area | What it covers | Earliest milestone |
| --- | --- | --- | --- |
| Kernel debug & diagnostics | `log`, `debug` | structured log, panic dumps, backtraces, tracing, self-tests, debugger integration | M3.5-B |
| Boot & recovery | `system` | boot configuration, boot log, recovery environment, power/restart | M5/M5U |
| Shell & core utilities | root command + areas | structured pipelines; list/show/copy/move/find style operations | M5U |
| Process & object tools | `process`, `object` | processes, threads, objects, handles, rights, resource usage | kernel view M3.5; native tools M5U |
| Filesystem & files | `fs` | roots, mounts, file operations, case policy, name escapes, path projection | M4/M4S; user tool M5U |
| Permissions & security | `access` | capabilities, profiles, authority receipts, accounts, audit | M5U |
| RegCube tools | `cube` | browse, history, rollback, transactions, schemas, explicit mounts | M9 |
| Between-OS tools | `compat` | binary classification, execution profiles, compatibility grants, Bridge inspection, compatibility tests | M6–M8 |
| Network tools | `net` | interfaces, addresses, DNS, connectivity, sockets, packet capture, network capabilities | M10 |
| Device & hardware tools | `device` | PCI/USB/device inventory, driver status, CPU, timer, memory, power hardware | data begins earlier; user tool M5U+ |
| Developer tools | `debug` plus host `tools/` | compiler target, ABI headers, debugger, profiler, cross-build and symbolization | M3.5-B host tools; native developer tools M5U |
| Packages & updates | `system` / future package surface | installation, removal, signing, updates | after M8 |
| Desktop & apps | `graphics` | displays, surfaces, compositor, terminal, settings, file manager | M11+ |
| System lifecycle | `driver`, `service` | inspect/start/stop/restart long-lived drivers and system services | M5U+ |

System lifecycle is intentionally distinct from process management. A service or driver may have lifecycle, policy, restart, capability, and dependency semantics that should not be reduced to a generic process entry.

## Roadmap placement

Existing architectural milestone numbers remain stable. New substrate is inserted with submilestones or later milestones instead of renumbering the established ABI/compatibility sequence.

### M3.5-A — physical-frame reuse

This is the first M3.5 change.

It adds the physical-frame return/free stack required before task stacks may be created and destroyed honestly.

No scheduler work begins before this proof exists.

### M3.5-B — debug foundation

Required before task switching:

1. structured kernel event ring;
2. panic/fatal backtrace support;
3. QEMU/GDB host tooling;
4. named self-test harness.

The interactive serial debug console is deferred.

### M3.5-C — task execution foundation

Only after M3.5-A and M3.5-B:

- task objects;
- task stacks;
- guard pages;
- saved CPU context;
- runnable state;
- deterministic scheduler/context switching.

### M4 — VFS namespace and RAM filesystem

M4 establishes FreeWorld's VFS/mount/root model and an in-memory filesystem.

### M4S — storage substrate

M4S adds the first real storage vertical slice:

~~~text
virtio-blk
    ↓
FW_BLOCK_DEVICE
    ↓
GPT / MBR partition discovery
    ↓
FAT32
~~~

Planned filesystem progression:

~~~text
RAMFS
  ↓
FAT32 read/write
  ↓
ext4 read-only
  ↓
NTFS read-only
  ↓
exFAT read/write
  ↓
ext4 write/journal semantics
  ↓
NTFS write
~~~

Btrfs, XFS, and ZFS are later work.

### M5 — user mode and callgate

Ring 3, FreeWorld-native process launch, native ELF loader, startup record, and callgate.

### M5U — native userland and administration protocol

M5U establishes:

- administration stream framing;
- the first `world` client;
- structured shell/pipeline behavior;
- the first user-mode system tools;
- authority receipts.

### M6–M8 — compatibility

- M6: ELF / LinuxFacet;
- M7: PE / WinFacet;
- M8: FW_MODULE / Bridge ABI.

Native FreeWorld administration remains independent of those facets.

### M9 — RegCube

Native `world cube` tools can then expose RegCube transactions, history, rollback, and schemas.

### M10 — networking

M10 is a controlled vertical slice:

~~~text
virtio-net
    ↓
Ethernet
    ↓
ARP + IPv4
    ↓
UDP
    ↓
TCP
    ↓
FW_SOCKET objects
    ↓
libfw networking API
    ↓
world net
~~~

IPv6, DHCP, DNS, and broader hardware support layer on afterward.

### M11 — graphics

M11 begins with graphics/input substrate, not a full desktop:

~~~text
firmware framebuffer / GPU surface
    ↓
FW_DISPLAY / FW_SURFACE objects
    ↓
input objects
    ↓
basic compositor
    ↓
windowing protocol
    ↓
desktop and graphical tools
~~~

Hardware-accelerated GPU drivers are later work.

# Locked decisions

## FW-ADMIN-01 — Native administration record framing

FreeWorld administration uses a FreeWorld-owned binary framing format.

JSON, CBOR, protobuf, or other external encodings may be supported as import/export formats, but they are not the native administration ABI.

### Stream header

The stream carries its magic/version framing once at stream start, not in a large repeated header on every record.

The stream header identifies at least:

- FreeWorld record-stream magic;
- protocol version;
- stream kind;
- framing flags.

The exact byte layout is deferred to M5U implementation work.

### Record envelope

Each record carries only the fields necessary for that record, including at least:

- schema identifier;
- flags;
- sequence where applicable;
- payload length;
- typed payload.

Integers in the v1 framing are canonical little-endian.

Payload fields use stable numeric field identifiers and explicit type/length information so readers can skip fields they do not understand.

Readers must enforce:

- a maximum record size;
- a maximum nesting depth;
- schema-specific field limits.

Malformed or over-limit records are rejected.

### Handles are out-of-band capabilities

A handle must never travel in a record as a plain numeric value.

A numeric handle copied into another process is meaningless at best and a forged-authority attempt at worst.

When a structured message transfers authority, handle fields travel through the channel's explicit handle-passing mechanism. The receiving process gets a valid receiving-side handle created by the kernel/object channel machinery.

Records may contain non-authoritative identifiers for display or correlation, such as object IDs or receipt IDs, when their schema explicitly defines them as identifiers rather than handles.

### Path/name fields

Record framing must not assume every path/name is UTF-8.

Path/name records carry enough information to preserve FreeWorld's native filename encoding model, including opaque Linux byte names and Windows-compatible WTF-8-style round trips where required.

## FW-DRIVER-01 — Driver placement

FreeWorld does not lock itself into either all-kernel drivers or all-userspace drivers.

The rule is:

> The kernel owns dangerous mechanisms. Drivers move behind service boundaries when the service/IPC/capability substrate is mature enough to isolate them honestly.

Kernel-owned mechanisms include:

- interrupt routing;
- DMA authorization;
- IOMMU mappings;
- MMIO mappings;
- port-I/O authority;
- device handles;
- memory pinning.

### M4S storage

The first `virtio-blk` path may begin kernel-side because it arrives before the mature driver-service substrate and is itself required to bootstrap real storage.

The block-device interface must nevertheless be designed so the driver can later move behind a service boundary without changing VFS semantics.

### M10 network driver experiment

M10 deliberately makes `virtio-net` the first serious driver-as-a-service experiment.

QEMU CI for that experiment must enable an emulated IOMMU.

Without an IOMMU, a userspace/service driver with DMA access could still write arbitrary physical memory, so the experiment would not prove meaningful driver isolation.

The target architecture is:

~~~text
kernel
  device capabilities
  MMIO / DMA / IRQ primitives
        │
        ▼
network driver service
  virtio-net
        │
        ▼
network protocol service
  Ethernet / IP / UDP / TCP
        │
        ▼
FW_SOCKET objects
~~~

The result of this experiment informs later decisions about moving storage, USB, GPU, and other drivers behind service boundaries.

## FW-SHELL-01 — Native shell and pipeline semantics

The canonical command form is:

~~~text
world <area> <verb> [target] [options]
~~~

FreeWorld may use familiar punctuation such as `|`.

The difference is semantic: a pipe carries a declared stream kind, not implicitly formatted text.

Example conceptually:

~~~text
world process list
    | filter state == running
    | fields id name memory
~~~

The first stage produces typed process records. Filtering and field selection operate on record fields, not terminal text.

### Explicit conversion

Conversions between stream kinds are explicit.

Examples:

~~~text
RECORD -> BYTE    render/encode
BYTE   -> RECORD  parse/decode
EVENT  -> RECORD  explicit event projection if supported
~~~

A command changing its human-readable spacing must not break a downstream RECORD pipeline.

### Shared expression grammar

Commands such as:

~~~text
filter state == running
~~~

are not allowed to invent tool-specific mini-languages.

M5U defines one shared expression/filter grammar for structured record operations. Filtering, field selection, comparisons, and related structured shell behavior build from that shared grammar.

The exact syntax is an M5U implementation detail.

## FW-AUDIT-01 — Authority receipts and bounded delegation history

Authority visibility belongs to the runtime/protocol, not to hand-written behavior in every tool.

A privileged operation may emit an authority receipt describing at least:

- operation;
- current object identifier;
- authorizing handle reference within the current process context;
- rights available;
- rights actually consumed;
- receipt/provenance identifier.

A handle entry stores only a bounded provenance reference, not its complete ancestry.

### Rights history

Delegation records may contain fields such as:

- receipt ID;
- parent receipt ID;
- tick/time reference;
- source object ID;
- source process;
- destination process where relevant;
- rights before;
- rights after;
- operation type.

The exact in-memory record layout is deferred.

### Bounded storage

Delegation/audit history is bounded.

Initial implementation targets may use bounds such as:

- a global record cap;
- a global memory cap;
- per-process quotas.

Exact numbers are implementation policy, not architecture.

### Anti-flood separation

Local duplicate activity must not be able to cheaply erase evidence of cross-process delegation.

Transfers between processes are tracked separately from local duplicate churn, or receive independent quotas/reservation.

A process that floods its local authority history should consume its own quota before displacing unrelated security-relevant history.

### Truncation is explicit

`world access trace <handle>` must never present the oldest surviving receipt as if it were the true origin when older history has rolled off.

It reports an explicit condition such as:

~~~text
history truncated
~~~

and distinguishes the oldest available receipt from the actual origin.

### Durable audit

When M9 RegCube/audit storage exists, selected authority events may be persisted by policy.

Permanent infinite delegation genealogy is not required. Durable audit retention is configurable and bounded by system policy.

## FW-EVENTS-01 — Shared event table

Kernel, userspace libraries, host tools, log renderers, and `world log` derive event identities from one source-of-truth event table.

Planned repository form:

~~~text
kernel/events.def
~~~

or an equivalent generated-source location chosen during M3.5-B.

The event table defines stable numeric IDs and symbolic names, for example:

~~~text
BOOT_BEGIN
BOOT_COMPLETE
FRAME_ALLOC
FRAME_FREE
HANDLE_CREATE
HANDLE_DUPLICATE
HANDLE_CLOSE
RIGHTS_DENIED
TIMER_CALIBRATED
NMI_RECEIVED
PAGE_FAULT
KERNEL_PANIC
~~~

From the same source, the build may generate:

- kernel Rust constants;
- `libfw` definitions;
- host log-decoder tables;
- `world log` schemas;
- documentation.

The kernel and decoder must never maintain independent hand-written mappings for the same event IDs.

## Structured kernel logging

M3.5-B replaces raw diagnostic strings as the primary log representation with a fixed-size structured event ring.

A normal event record is numeric/fixed-width data such as:

~~~text
sequence
tick
cpu
subsystem
level
event_id
arg0
arg1
...
~~~

Serial output becomes one renderer/sink of that event stream rather than the authoritative log itself.

### NMI-safe path

NMI logging must remain allocation-free and lock-free.

An NMI records a compact predefined event, for example `NMI_RECEIVED`, without constructing formatted strings or taking the normal serial/logging lock.

### Panic record exception

Rust panic information contains a text message, so panic records have one explicit bounded text exception.

A panic/fatal record may contain:

- fixed numeric event fields;
- register/fault information;
- backtrace addresses;
- one fixed 256-byte panic-message buffer;
- message length/truncation state.

The panic path must not allocate to capture this string. Messages longer than the fixed field are truncated and marked as such.

Other ordinary kernel event records remain numeric/fixed-layout.

## M3.5-B debug foundation

The following are required before scheduler/context-switch work begins.

### Structured kernel event ring

- fixed-size bounded ring;
- shared FW-EVENTS-01 IDs;
- normal and NMI-safe write paths;
- serial as a renderer/sink;
- later user-mode access through capability-controlled tools.

### Panic/fatal dump with backtrace

On panic or fatal exception:

~~~text
disable maskable interrupts
    ↓
mark/freeze panic context
    ↓
register/fault dump
    ↓
frame-pointer backtrace
    ↓
recent structured events
    ↓
bounded panic message
    ↓
halt
~~~

Debug kernel builds retain frame pointers as required for the initial stack walker.

### QEMU/GDB host tooling

The repository gains a `tools/` location for repeatable host-side utilities such as:

- QEMU launch/debug script;
- GDB launcher/configuration;
- serial-log parser;
- kernel-address symbolizer;
- image helper/build scripts.

QEMU's GDB server is used rather than creating a custom kernel debugger before one is needed.

### Named self-test harness

Existing per-milestone self-tests are converted into named tests whose output identifies the exact test that passed or failed.

This is kernel-side infrastructure and does not require user mode.

### Interactive serial console

An interactive kernel debug console is intentionally deferred. GDB, structured logs, panic dumps, and named tests provide greater value before task switching.

## Storage foundation

Real file tools depend on block-device and partition objects, not merely a VFS parser.

The storage stack begins:

~~~text
FW_BLOCK_DEVICE
    ↓
partition scanner
    ├─ GPT
    └─ MBR
    ↓
filesystem driver
~~~

The FreeWorld filesystem tools operate on FreeWorld objects and views regardless of whether the underlying filesystem is FAT32, ext4, NTFS, exFAT, or a later driver.

Native path translation is exposed under `world fs` and can render a FreeWorld path using native, Windows, or Linux spelling where a valid projection exists.

## Network foundation

M10 networking is capability-oriented.

Network authority is represented by FreeWorld objects/handles rather than ambient access to a global socket namespace.

The first QEMU vertical slice is intentionally narrow and measurable. Later features such as DHCP, DNS, IPv6, packet capture, firewall policy, and real NIC drivers extend that foundation.

## Graphics foundation

M11 separates display/surface/input mechanisms from desktop policy.

The initial graphics milestone proves display and surface objects and a basic compositor/window protocol before attempting a full desktop environment or hardware-accelerated GPU stack.

`world graphics` is the native administration view of those objects; graphical settings/diagnostic applications may use the same protocol directly.

## Authority receipts in tools

Authority reporting is a runtime feature.

A tool does not individually invent how to print capabilities.

A common mode such as:

~~~text
world --authority ...
~~~

may request authority receipts from the protocol/tool runtime.

A receipt can distinguish:

- rights present on the handle;
- rights consumed by the operation;
- object identity;
- provenance/delegation state;
- whether available history is complete or truncated.

This keeps FreeWorld's capability model inspectable without turning raw handles into transferable numeric tokens.

## Originality and compatibility boundary

FreeWorld's native administration architecture is its own interface.

Nothing in this document requires cloning:

- Unix utility names;
- Unix text-pipeline semantics;
- Windows administrative utilities;
- Windows registry tools;
- platform-specific shell behavior.

At the same time, generic operations such as listing, copying, filtering, starting, stopping, mounting, inspecting, and showing are normal computing concepts and may use clear ordinary verbs.

Compatibility tools inspect or configure LinuxFacet/WinFacet behavior through FreeWorld objects; they do not make the native FreeWorld tool model a copy of either environment.

## Implementation details intentionally deferred

The architecture above is settled. The following are implementation details to be frozen at the milestone that first needs them:

1. **Exact FW-ADMIN-01 byte framing.** Magic bytes, concrete field widths beyond the locked semantics, schema encoding, maximum record size, and nesting limits are defined in M5U.

2. **Driver placement by device class.** FW-DRIVER-01 defines the mechanism/service rule and M10 network experiment. Later drivers are placed based on measured isolation, latency, complexity, and service maturity.

3. **Exact shell grammar.** FW-SHELL-01 locks structured semantics and one shared expression language. M5U freezes the lexical/parser grammar.

4. **Delegation/audit capacity.** FW-AUDIT-01 locks bounded, quota-aware, truncation-explicit history. Exact record/memory quotas and durable retention limits are policy defined when the implementation exists.

These are not architectural unknowns. They are deliberately deferred constants, encodings, and policies.

## Decision summary

Locked decisions:

- native tools use a structured FreeWorld administration protocol;
- `world` is the canonical CLI client, not the system API;
- the `world` top-level areas are fixed by this document;
- streams are RECORD, BYTE, or EVENT;
- handles transfer out-of-band through capability-aware channel mechanisms;
- storage gains M4S before real disk filesystem tools;
- native userland/tools gain M5U;
- networking is M10;
- graphics is M11;
- M10 tests userspace driver isolation with an IOMMU;
- shell filtering uses one shared M5U expression grammar;
- audit/delegation history is bounded, quota-aware, and explicit about truncation;
- event IDs come from one generated source of truth;
- ordinary events remain numeric/fixed-layout;
- panic events receive one bounded 256-byte text field;
- M3.5-B debug infrastructure lands before scheduler/context switching.

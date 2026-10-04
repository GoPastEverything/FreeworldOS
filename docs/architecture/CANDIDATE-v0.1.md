# FreeWorldOS Candidate Architecture v0.1

**Status:** recovery baseline, not final authority.

## Founding law

**Compatibility at the edges. Originality inside.**

For every compatibility feature, ask: **what behavior does the binary require at the interoperability boundary, and what is the best FreeWorldOS-native mechanism underneath it?**

## Native core

Foreign software is projected onto FreeWorld-native concepts:

```text
PE/Windows ---- WinFacet -----\
                               > FW_PROCESS / FW_THREAD / FW_MODULE
ELF/Linux ----- LinuxFacet ---/  FW_OBJECT / FW_VFS / REGCUBE / FW_SECURITY

FreeWorld native --------------------------^ 
```

## Execution profile

FreeWorldOS keeps these independent:

- environment: FreeWorld / Windows / Linux
- image format: FreeWorld / PE / ELF
- ABI: FreeWorld64 / Microsoft x64 / System V x64 / later AAPCS64
- CPU architecture: x86_64 first, ARM64 designed-for but deferred

## Filesystem rule

FreeWorldOS has one native mount/object graph containing root/volume objects. Each process receives a namespace projection.

- WinFacet may render a root as `C:\\...`
- LinuxFacet may render it as `/...` or a mount point
- FreeWorld-native tools may render tagged paths such as `<C:>Games\\...` and `</>home/...`

**Path syntax is presentation; object identity is native.** Cross-facet visibility is explicit policy, not automatic.

Disk formats are a separate layer. NTFS/exFAT/ext4/etc. are filesystem-driver concerns, not namespace identity.

## RegCube rule

**RegCube stores state. VFS stores files.**

RegCube's core axes are identity, scope, and revision. Properties/schema/type are cell address metadata. Compatibility layers can project foreign state interfaces onto RegCube. Synthetic files may be backed by state explicitly, but arbitrary files never silently become RegCube entries.

## RTOS direction

The initial kernel is RTOS-influenced rather than a desktop monolith: deterministic scheduling, explicit task state, bounded kernel mechanisms where practical, capability-oriented object access, and clear separation between mechanism and compatibility policy.

The exact scheduler class, preemption model, SMP policy, and real-time guarantees remain open until timing/interrupt infrastructure exists and can be measured.

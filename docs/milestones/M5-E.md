# M5-E — One user leaf in an inactive process root

Status: implementation branch after frozen M5-D.

M5-E installs exactly one 4 KiB user-data leaf into an inactive
ProcessObject-owned PML4 and reclaims all private mapping frames at final
process-object destruction. It does not load that root into CR3.

## Ownership

The production ownership chain is:

```text
ProcessObject
  -> ProcessAddressSpace
        -> PML4 root frame (M5-D)
        -> optional InactiveUserLeaf
              -> PDP (L3) table frame
              -> PD  (L2) table frame
              -> PT  (L1) table frame
              -> 4 KiB user leaf data frame
```

The one-leaf mapping is constructed while the ProcessObject is uniquely
owned, before Arc/handle publication. The published object has no concurrent
mapping-mutation API.

## Mapping

M5-E accepts a page-aligned lower-canonical-half user VA and user
permissions. Writable+executable and device-cache user mappings are refused.

The inactive PML4 initially has an empty lower half. M5-E builds one
private chain:

```text
PML4[user_index] -> PDP[user_index] -> PD[user_index] -> PT[user_index]
                                        -> owned user leaf frame
```

All three ancestors are PRESENT, WRITABLE and USER_ACCESSIBLE.

A normal user RW mapping ends at a PRESENT, WRITABLE, USER_ACCESSIBLE, NX
leaf. Data is zero-initialized through the higher-half physical direct map.

There is still only one mapping per ProcessAddressSpace. A second mapping
attempt fails rather than growing an undocumented VM subsystem.

## All-or-nothing construction

All four frames for the lower-half mapping are allocated and zeroed before
the new top-level PML4 entry is published.

If any frame allocation fails, already allocated frames are returned through
the recycler without publishing a partial subtree. A failure to create or
insert the whole ProcessObject also drops its previously owned frames.

## Destruction

Before reclaiming, M5-E verifies that all four PTE links still have exactly
the expected physical addresses and flags, and that no extra lower-half
PML4 or child entries exist.

The inactive subtree is first detached from its PML4 root. The user data
frame and three private intermediate table frames are then returned to the
recycler. Finally, M5-D's existing empty-lower-half root teardown returns
the PML4 root.

Neither traversal nor reclamation touches PML4[256..511], so shared
higher-half kernel structures are never reclaimed by process teardown.

Unexpected additional mappings or a corrupted chain fail closed.

## QEMU proof

After M5-D's existing test, the combined CI image:

1. Attempts a higher-half user mapping through process construction and
   requires UserMappingOutsideLowerHalf without publishing a handle.
2. Creates one process with a user RW/NX mapping at
   0x0000_5000_2000_0000, plus a second empty process.
3. Inspects the actual inactive page-table links and permission flags.
4. Writes and reads a test value through the kernel physical direct map,
   leaving the process PML4 inactive.
5. Proves the second process's lower half remains empty, and that both
   processes retain the same shared higher-half kernel mappings.
6. Closes both handles, drops their last references, and requires exactly
   six newly returned frames: mapped process's root + three private tables +
   user leaf, plus the empty peer's root.

Expected proof:

```text
FreeWorldOS: M5-E inactive leaf: root=0x... peer=0x... virtual=0x500020000000 data=0x... ancestors=user leaf=rw_nx peer_lower=empty kernel_half=shared active_cr3=unchanged
FreeWorldOS: SELFTEST PASS name=m5e.process_user_leaf
FreeWorldOS: M5-E process user-leaf self-test: passed inactive_mapping=ok user_leaf=1 ancestors=3 user_writable=ok leaf_nx=ok isolated_peer=ok rollback_rejected=ok frames_returned=6 cr3_switch=off user_execution=off callgate=off
```

## Explicit non-claims

This is not an ELF loader, a user task, or a general virtual memory manager.
There is no process CR3 switch, no user execution from the new process root,
no user scheduling or preemption, no PCIDs, no callgate, and no syscall ABI.

The next VM slice must introduce safe multi-page mapping and full lower-half
ownership/reclamation before any process address space becomes runnable.

# M5-D — Process-owned inactive address-space root

Status: implementation branch after frozen M5-C.

M5-D turns the existing ProcessObject placeholder into a real kernel object that owns a distinct x86-64 address-space root.

It does **not** load that root into CR3 and does not execute user code from it.

## ProcessObject ownership

A ProcessObject now owns:

~~~text
ProcessIdentity
  +
ProcessAddressSpace
      |
      v
distinct PML4 physical frame
~~~

The root frame remains owned for the entire lifetime of the process object.

Closing the final process handle drops the ProcessObject, then the ProcessAddressSpace owner returns the root frame to the existing physical-frame recycler.

## Root construction

Creation allocates one 4 KiB frame for the new PML4 and explicitly zeroes the full page.

The root is then populated as:

~~~text
PML4[0..255]
    private process lower half
    explicitly empty

PML4[256..511]
    copied from the currently active kernel PML4
~~~

Only top-level higher-half entries are copied.

The referenced lower-level kernel page tables remain shared. Therefore changes inside an already-shared higher-half PML4 slot remain visible through those shared descendants.

M5-D intentionally does not copy any lower-half entry from the active kernel root, even if the active root retains empty intermediate tables created by earlier temporary user mappings.

## Kernel topology limitation

M5-D snapshots the set of higher-half PML4 entries at process creation.

If a later kernel feature creates a **new previously-unused higher-half PML4 slot**, existing process roots will not automatically gain that new top-level entry.

Before FreeWorld permits arbitrary late kernel virtual-region creation, it must provide one of:

- a kernel PML4 template propagated to every process root;
- explicit update of all live process roots;
- or a fixed kernel virtual layout whose required top-level entries are established before process creation.

M5-D does not claim that mechanism already exists.

## No process user mappings yet

The process lower half is private but empty.

M5-D does not provide APIs to install user leaf mappings into the inactive process root.

This is deliberate because once lower-level process page tables exist, address-space destruction must recursively reclaim:

- user leaf frames owned by the process;
- lower-level paging-structure frames;
- while never freeing shared higher-half kernel structures.

M5-D destruction therefore refuses to free a process root whose lower half is nonempty.

That invariant is a tripwire for the next mapping slice.

## No CR3 switch

The process root is inspected only through FreeWorld's higher-half physical-memory direct map.

M5-D does not:

- load process CR3;
- execute on the process address space;
- perform TLB switching;
- use PCIDs;
- schedule a user task.

The M5-C shared RSP0 stack is therefore not preempted or reused by a process execution path in this slice.

## Handle integration

FwObject gains a Process variant.

The normal handle table can create and inspect process handles.

Final-close behavior is currently allowed because an M5-D ProcessObject cannot be Running and its address-space root is inactive.

## M5-D CI proof

The combined self-test creates two FreeWorld64 process objects through the real handle table.

It requires:

- both process object IDs are nonzero and distinct;
- both execution profiles are preserved;
- two different process PML4 frames are allocated;
- neither process root equals the active kernel CR3 root;
- both processes report the same active kernel root;
- every lower-half PML4 entry in each process root is unused;
- every higher-half entry has the same address and flags as the active kernel root;
- no copied higher-half PML4 entry carries USER_ACCESSIBLE.

Then both process handles are closed.

The physical-frame recycler must report exactly two additional returned frames, proving both process root frames were reclaimed.

Expected markers:

~~~text
FreeWorldOS: M5-D process roots: first=0x... second=0x... kernel=0x... distinct=ok lower_private_empty=ok higher_shared=ok higher_user=off
FreeWorldOS: SELFTEST PASS name=m5d.process_address_space
FreeWorldOS: M5-D process address-space self-test: passed process_object=ok own_pml4=ok lower_half=private_empty higher_half=kernel_shared root_reclaim=2 cr3_switch=off user_execution=off preemption=off callgate=off
~~~

## Deferred

M5-D does not implement:

- process-local user leaf mappings;
- recursive process page-table teardown;
- CR3 switching;
- TLB/PCID policy;
- a ProcessObject-owned task;
- user stack lifetime;
- timer preemption in user mode;
- user exception delivery;
- ELF startup;
- FwStartInfoV1;
- STAR/LSTAR/SFMASK;
- SYSCALL/SYSRET;
- native call table;
- native callgate.

The next address-space slice must preserve higher-half sharing while adding process-owned lower-half paging structures and a complete reclamation rule before any process CR3 becomes runnable.

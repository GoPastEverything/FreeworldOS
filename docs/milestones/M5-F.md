# M5-F — Controlled process CR3 round-trip

Status: implementation branch on M5-E.

M5-F proves that the process-owned PML4 built in M5-D and populated with
one private user leaf in M5-E can actually be loaded into CR3 and used to
access that leaf by its lower-half virtual address. This is a **CPL0-only**
single-CPU proof, not execution of a user task.

## Hardware sequence

A single CI-only assembly function runs with maskable interrupts disabled:

~~~text
save RBX / old CR3
MOV CR3, process_root
observe CR3
write pattern via process virtual address
read pattern via process virtual address
MOV CR3, saved_kernel_root
observe restored CR3
restore RBX
return to kernel Rust
~~~

No Rust function call, allocation, memory-manager lock, scheduler handoff,
logging, or user CPL3 transition occurs between loading and restoring CR3.

CR3 reloads naturally invalidate relevant non-global translations; PCID
policies are not added in this slice.

## Preconditions

- Hold a strong reference to the ProcessObject during the entire switch.
- Validate its inactive root and its one-leaf paging chain via the direct map.
- Require ancestors and leaf user-accessible, leaf writable and NX.
- Prove that the user VA is **not mapped** in the currently active kernel root.
- Require entry outside interrupt context; mask normal interrupt delivery.
- The process root must contain the same higher-half kernel image, code,
  stack, direct map and other kernel-owned paging structures as the kernel root.

The active stack and return address are in the shared higher half. NMI/MCE
exception handling still relies on the existing dedicated kernel IST mapping.

## CI proof

After the earlier M5-E proof, create a mapped process and a second empty
process. Both remain alive via strong object references.

The mapped process has a U+RW/NX page at:

~~~text
0x0000_5000_2000_0000
~~~

The test asserts:

- kernel CR3 before != process root;
- observed CR3 while executing the memory access == process root;
- the write and read through the user VA match the test pattern;
- the original kernel CR3 is restored before returning to Rust;
- the physical-frame direct-map read after restoration sees the same pattern;
- kernel root does not map the tested user VA;
- the peer process lower half remains empty;
- both process higher halves still match the shared kernel root.

After the CR3 round-trip, both handles and their final strong references
are released. CI requires the frame recycler to receive six frames:
five from the mapped process and one from the empty peer.

Expected markers:

~~~text
FreeWorldOS: M5-F CR3 roundtrip: kernel_before=0x... process=0x... kernel_after=0x... virtual=0x500020000000 value=0x4d354650524f4345 peer_root=0x... kernel_leaf_absent=ok
FreeWorldOS: SELFTEST PASS name=m5f.process_cr3_roundtrip
FreeWorldOS: M5-F process CR3 self-test: passed process_cr3=loaded user_virtual_rw=ok kernel_cr3=restored kernel_root_leaf=absent physical_direct_map=match peer_isolated=ok frames_returned=6 if=masked cpl0=only preemption=off callgate=off
~~~

## Not yet implemented

M5-F does not enter CPL3 on the process root; it does not schedule or
preempt a user task; it does not switch TSS RSP0 per process; it does not
build a general VM/multi-page allocator, syscall interface, native call
table, or callgate. The existing M5-C one-off shared-RSP0 privilege
round-trip remains separate and unchanged.

The process root is loaded only for this isolated assembly proof.

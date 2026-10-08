# M5-C — Controlled CPL3 entry and return

Status: implementation branch after frozen M5-B.

M5-C performs the first actual privilege transition in FreeWorldOS.

It is deliberately **not** a user task, not a process, and not the native callgate.

## Purpose

M5-A proved the lower-half user / higher-half kernel address split.

M5-B installed:

- DPL3 user code/data descriptors;
- future user selectors SS=0x1b and CS=0x23;
- a dedicated higher-half TSS RSP0 privilege-transition stack.

M5-C now proves those pieces work together on hardware by performing one controlled:

~~~text
CPL0 -> IRETQ -> CPL3 -> INT -> TSS.RSP0 -> CPL0
~~~

round-trip.

## User mappings

The test creates two temporary lower-half user mappings in the current kernel page table:

~~~text
0x0000_4000_1000_0000  user code
0x0000_4000_1000_1000  user stack
~~~

The code page is staged as U+RW only long enough to copy the fixed test stub, then unmapped and remapped as U+RX.

The stack page is U+RW and NX.

This proves the existing page-table hierarchy is actually user-accessible from CPL3, not merely marked correctly at the leaf while being read from CPL0.

## User stub

The fixed CPL3 code performs only:

~~~text
mov rax, USER_MARKER
push rax
int 0xf1
ud2
~~~

The UD2 is unreachable if the one-shot return path works.

The pushed marker proves the user stack is writable from CPL3.

The instruction following the software interrupt gives an exact expected user RIP in the hardware return frame.

## Entry

The kernel enters with maskable interrupts disabled.

A small assembly routine saves the current kernel call frame, then builds an IRETQ frame containing:

- user SS = 0x1b;
- lower-half user RSP;
- RFLAGS with IF clear;
- user CS = 0x23;
- lower-half U+RX user RIP.

IRETQ performs the actual CPL0 -> CPL3 transition.

M5-C does not enable timer delivery during the user interval.

## Test-only return vector

Vector 0xF1 exists only when `m5c-ci-self-test` is compiled.

Its IDT descriptor:

- points at a hand-written ring-0 entry;
- has DPL3 so CPL3 may execute `INT 0xF1`;
- is not present as a public FreeWorld ABI in ordinary kernels.

This vector is **not** the native callgate and does not reserve the future syscall mechanism.

## TSS RSP0 proof

On the privilege-changing software interrupt, the CPU must switch from the lower-half user stack to the M5-B TSS RSP0 stack.

The hand-written return entry pushes all fifteen GPRs on top of the hardware privilege-transition frame and passes the resulting 160-byte frame to Rust.

The verifier requires:

- the frame lies wholly inside the dedicated M5-B RSP0 stack;
- saved CS == 0x23 and carries RPL3;
- saved SS == 0x1b and carries RPL3;
- saved RIP is the byte immediately after `INT 0xF1`;
- saved RSP is the user stack pointer after the marker push;
- saved RAX still equals USER_MARKER;
- the marker is readable at the saved U+RW user RSP;
- saved IF remains clear;
- the return handler itself executes with CPL0 CS.

No LAPIC EOI is involved because this is a software interrupt.

## Return to the original kernel call frame

M5-C intentionally does not IRETQ back to user mode after the proof.

The CI-only gate consumes a one-shot arm, verifies the hardware frame, restores the frozen kernel data selectors, abandons the temporary RSP0 handler frame, restores the saved kernel RSP and jumps back to the original assembly continuation.

The original Rust caller then verifies:

- the gate was consumed exactly once;
- interrupt depth returned to zero;
- IF remains disabled;
- kernel SS is restored;
- the observed frame address was on RSP0.

The two temporary user mappings are then removed and both physical frames are returned to the recycler.

## CI markers

Expected markers include:

~~~text
FreeWorldOS: M5-C ring3 entry: rip=0x400010000000 rsp=0x400010001ff0 cs=0x23 ss=0x1b if=off return_vector=0xf1
FreeWorldOS: M5-C gate: vector=0xf1 frame=0xffff... user_cs=0x23 user_ss=0x1b user_rip=0x40001000000d user_rsp=0x400010001fe8 rsp0=ok marker=ok
FreeWorldOS: M5-C ring3 return: frame=0xffff... rsp0_stack=ok user_rip=ok user_rsp=ok user_stack_marker=ok cpl3=observed cpl0=restored kernel_ss=restored frames_recycled=ok
FreeWorldOS: SELFTEST PASS name=m5c.ring3_roundtrip
FreeWorldOS: M5-C privilege round-trip: passed iretq_to_ring3=ok user_rx=ok user_rw_nx=ok dpl3_int_gate=test_only tss_rsp0=used ring0_return=ok scheduler=off user_task=off callgate=off
~~~

## What M5-C does not claim

M5-C does not implement:

- a user TaskObject;
- a ProcessObject-owned user address space;
- a separate CR3;
- scheduler entry/exit for CPL3 tasks;
- timer preemption while in CPL3;
- user exception delivery;
- persistent user stacks;
- ELF loading;
- FwStartInfoV1;
- STAR/LSTAR/SFMASK;
- SYSCALL/SYSRET;
- the FreeWorld native call table;
- the native callgate.

The dedicated vector 0xF1 is CI-only proof machinery and must not become a compatibility or application ABI.

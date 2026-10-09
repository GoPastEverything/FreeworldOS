# M5-G — One-shot CPL3 execution on a process PML4

Status: implementation branch; requires exact-head CI verification.

M5-G combines the M5-C controlled privilege entry with M5-F's real
process-owned CR3, without creating a schedulable userspace task or a
FreeWorld callgate.

## One-leaf constraint

M5-E allows only ONE process-owned user leaf. M5-G constructs that leaf
at 0x0000_5000_3000_0000 with U+RX permissions and stages a short
instruction stub through its physical direct-map alias while the process
root is inactive.

The stub is exactly:

- MOVABS RAX, M5-G marker;
- INT 0xF2;
- UD2 (must remain unreachable).

The user RSP points inside the same mapped page but the stub performs
NO push, pop, call, or write to this user stack. This does NOT prove a
writable userspace stack or general user-program execution. The CPU uses
the TSS RSP0 for its CPL3-to-CPL0 interrupt frame.

## Entry

The M5-G assembly entrance:

1. saves kernel callee-preserved registers and the kernel stack RSP;
2. saves the FULL original CR3 value;
3. loads the ProcessObject PML4 into CR3;
4. builds the five-word CPL3 IRETQ frame
   (SS=0x1b, user RSP, RFLAGS=2, CS=0x23, user RIP);
5. executes IRETQ.

The entire CPL3 interval has IF clear, is single-CPU, and has no
scheduler handoff.

## Return

Vector 0xF2 is installed in the IDT at DPL3 ONLY in m5g-ci-self-test.
It is not a public ABI.

INT 0xF2 transitions to the dedicated bootstrap TSS RSP0 stack. Its
handwritten handler saves all 15 general-purpose registers, observes
the process CR3, and restores the FULL saved kernel CR3 **before**
calling any Rust routine.

The Rust verifier runs on the restored kernel CR3 and proves:

- the saved 160-byte frame is wholly on the M5-B RSP0 stack;
- saved user CS=0x23, SS=0x1b, and RPL3;
- saved RIP immediately follows the INT and points at UD2;
- saved user RSP matches the IRETQ input (no user-stack writes);
- saved RAX matches the instruction-stub marker;
- saved IF is clear and the handler itself runs at CPL0;
- observed process CR3 matches this ProcessObject's PML4;
- kernel CR3 is already restored before Rust verification.

It restores the kernel data selector/SS on RSP0, then the handler
abandons the temporary frame, restores the saved kernel call stack and
returns through the original call frame.

## Object lifetime and cleanup

The process remains held by a strong object reference during the
hardware transition.

After the round-trip, the inactive PML4 chain is validated again,
allowing only CPU-updated Accessed/Dirty bits per M5-F. The process
handle and its last strong reference are closed only after the kernel
CR3 is restored.

A mapped process and an empty peer return exactly six frames in total:
one leaf + three private ancestor tables + two distinct PML4 roots.

## CI gate

The combined self-test must include:

~~~text
FreeWorldOS: M5-G return gate: vector=0xf2 process_cr3=0x... user_cs=0x23 user_ss=0x1b user_rip=0x50003000000c user_rsp=0x500030000ff0 rsp0=ok kernel_cr3=restored marker=ok
FreeWorldOS: M5-G process ring3: kernel_before=0x... process=0x... kernel_after=0x... code=0x500030000000 frame=0xffff... cs=0x23 ss=0x1b vector=0xf2
FreeWorldOS: SELFTEST PASS name=m5g.process_ring3_cr3
FreeWorldOS: M5-G process ring3 self-test: passed process_cr3=active_in_cpl3 iretq=ok user_rx=ok user_stack_writes=off tss_rsp0=used kernel_cr3=restored_before_rust frame_reclaim=6 if=masked scheduler=off callgate=off
~~~

## Explicit limits

This is NOT a user TaskObject, a scheduler-selected process CR3, a
multi-page user executable or a working user stack. It does not enable
timer preemption across a shared RSP0, and does not create FwStartInfoV1,
SYSCALL/SYSRET, STAR/LSTAR, the native call table, or the callgate.

Only after this exact-head proof is green should later milestones add
a process-owned user stack, independent user task kernel stacks, and
full context-switch and trap handling.

# M5-K — one-process suspended frame and IRETQ resume

Status: **implemented and QEMU-verified**. PR #35 merged into main at
`4cf6eeceb6948d7123448c096a51c1409acea998`.
Exact implementation head: `25aefb5351399b76b52c29d67992ccbfd0f0f82d`.
GitHub Actions CI run: `38195219257` — completed successfully.
QEMU serial: `SELFTEST PASS name=m5k.process_suspend_resume`.
Base: `095598b68cb57972df04b5bd28852831fef69fcc` (merged M5-J).

## Goal

Run one test-owned ProcessObject on its own CR3, user leaves, and private
higher-half kernel stack. Enter CPL3 once, trap on 0xF2, retain the complete
160-byte saved frame in the process-owned kernel stack, return to the bootstrap
kernel call stack without freeing or modifying the saved frame, then resume
the original frame with IRETQ and trap on 0xF2 a second time. Finish by
restoring kernel CR3 and bootstrap TSS.RSP0 before releasing process ownership.

No scheduler choice, timer interrupt handoff, callgate, or public syscall ABI.

## Test stub and offsets

```asm
movabs rax, USER_MARKER    ; 10 bytes, offset 0
push rax                  ;  1 byte,  offset 10
int 0xF2                  ;  2 bytes, offset 11 (suspend)
int 0xF2                  ;  2 bytes, offset 13 (resume destination)
ud2                       ;  2 bytes, offset 15 (must not execute)
```

Code VA 0x500030000000 U+RX; stack VA 0x500030001000 U+RW,NX.
Entry RSP = 0x500030002000; after the one push = 0x500030001ff8.
The stub is exactly **17 bytes**. The first saved RIP is **0x50003000000d** (second INT); second saved
RIP is **0x50003000000f** (UD2). Verify the marker through the
kernel direct map after the **final** kernel CR3 restoration (one read only); verify that the user RSP
is unchanged between the two returns and the marker was not pushed twice.

## Frame ownership

The first handler must keep all 160 bytes of its 15-GPR + 5-word
hardware frame intact on the private kernel stack when returning to
the bootstrap kernel call stack. Its saved frame address and owner/root
must survive until resumption. While suspended, kernel CR3 is active;
process-owned PML4, both leaves and private stack remain live.

The original first frame must occupy precisely **[top-160, top)**.
Before resuming it, set TSS.RSP0 to **top-160**, while keeping the bootstrap
RSP0 saved. The second trap then owns **[top-320, top-160)**.
Assert both ranges are non-overlapping, wholly within A's private kernel
stack, and disjoint from the bootstrap RSP0 stack. Neither trap may
overwrite the original 160-byte frame before resumption.

An IRETQ-resume trampoline must activate A's CR3, restore the saved GPRs
and consume the **original** hardware IRETQ frame, without returning to
the stub's entry point. Reconcile TSS.RSP0 before the second trap so
the second hardware frame and 15-register save do NOT overwrite the
suspended frame. Check stack bounds for BOTH 160-byte frames. Do not
reassign TSS.RSP0 to the bootstrap stack during the suspension; restore
bootstrap RSP0 only after final return.

Rust verification must run only under restored kernel CR3. Maintain IF=0
through both user-mode intervals, no scheduler/preemption, and test-only
IDT vector 0xF2.

## Review checks

- Saved first RIP 0x50003000000d, second RIP 0x50003000000f.
- Both traps come from CPL3 with correct user CS/SS and observed process CR3.
- First saved frame is exactly [top-160, top); second is [top-320, top-160); TSS.RSP0 is switched to top-160 before resume.
- First saved frame belongs to A's private kernel stack and remains
  byte-for-byte valid until the resume trampoline consumes it.
- A's second interrupt frame fits the same private kernel stack without
  overwriting frame one; no bootstrap RSP0 frame.
- Kernel CR3 restored before any Rust callback in both return paths.
- Marker at 0x500030001ff8 unchanged; RSP stays 0x500030001ff8.
- Process lifetime spans suspension, resume and final return.
- Bootstrap RSP0 restored only after final interrupt return.
- Full frame recycling and prior M5-G/H/I/J regression tests still pass.
- CI exact-head QEMU evidence, independent review, then merge.

No claim of userspace task scheduling or a general process-resume API.

## Accepted QEMU observations (run 38195219257)

- Private kernel stack top: `0xffffa00000014000`.
- First 160-byte frame: `0xffffa00000013f60` = top−160.
- Second 160-byte frame: `0xffffa00000013ec0` = top−320.
- Saved first RIP / resume destination: `0x50003000000d`.
- Saved final RIP: `0x50003000000f`.
- Kernel CR3 restored before Rust; bootstrap RSP0 restored only after second return.
- User marker checked once via direct map after final CR3 restoration.
- Ten frames reclaimed; no timer-driven scheduling or callgate.

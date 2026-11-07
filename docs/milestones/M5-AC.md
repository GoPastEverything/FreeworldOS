# M5-AC — One-process native lifecycle (draft)

**Purpose:** Add a `Created → Running → Stopped(status) → explicitly reaped` process lifecycle using the M5-AA task-owned syscall stack. This deliberately does not schedule a user task or use the previous M5-G one-shot INT 0xF2 return gate.

## Implementation

- `fw-abi::FW_SYS_EXIT = 3`: native process exit, with RDI supplying status; this successful call **never returns** to ring 3.
- `ProcessObject::new_with_syscall_task` allocates the guarded 4-page stack and boxed task context before creating the process CR3. `ProcessObject::start_native` is a separate callable lifecycle API, not code hidden inside the QEMU self-test; the API enforces once-only start.
- `UserSyscallTask` owns the lifecycle state. It cannot be started twice. `process::reap_stopped(&mut Option<ProcessObject>)` refuses Created/Running and explicitly drops a stopped owner.
- `__freeworld_m5ac_enter` saves the supervisor stack continuation and original kernel CR3 **in task context**, loads the process CR3 and IRETQs into ring 3. The code page contains actual SYSCALL #1(41) then SYSCALL #3(7), followed only by an unreachable UD2 (no INT F2).
- On exit #3 the Rust dispatcher records the status and arms an exit flag in the same task context. The assembly entry skips SYSRET, restores kernel CR3 and GS, then restores the original supervisor stack and its preserved callee-saved registers. Stack memory is still live at this point.
- After the call returns to Rust, the owner moves to Stopped(7). Explicit reaping then destroys its address space and task stack, checking that ten owned physical frames were returned.

## Acceptance requirements

Exact-head CI must prove: created state; rejection of premature reap; real ring-3 calls #1 and #3; handler running on task-owned stack; exit status 7 observed by kernel without a user continuation/INT F2; MSRs, GS, TSS RSP0, CR3 restored before reaping; second start refused; stopped retained until explicit reap; ten frames returned; live process count and heap allocations unchanged; old M5-O–U and M5-AA–AB regressions still pass.

## Explicit limits

Feature-gated single process with IF masked. The bootstrap kernel call continuation is still involved as a non-scheduled parent: it is now **task-owned** rather than a global test slot. No time slicing or run queue, no general native ELF loader, no concurrent unmap safety, and no LinuxFacet/WinFacet. Exit code is a u64 proof; ABI semantics for exit codes beyond this fixture are not settled.

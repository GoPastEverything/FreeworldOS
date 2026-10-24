# M5-AB — Task-owned FreeWorld syscall dispatch

The M5-AA entry saves caller registers on the private task stack and forwards the native FreeWorld syscall number (RAX) and first two arguments (RDI, RSI) to the Rust dispatcher. It does not use the M5-O global syscall slots.

The feature-gated one-task acceptance stub, assembled from x86-64 instructions, performs four actual SYSCALLs before the separate INT 0xF2 test exit: number 1 with 41 -> 42; number 2 reads 20 bytes from 0x500030000080 via the preflighting page walker and returns 20; number 2 with unmapped pointer 0x500030003000 returns FW_ERR_USER_ADDRESS (-8193); unknown number 99 returns FW_ERR_NO_SUCH_CALL (-4098). The user stub compares full 64-bit RAX values, not EAX. The context records four invocations and independent success/bad-pointer/unknown counts; the final context and 160-byte test trap frame are checked before process reclamation.

The original M5-AA marker test remains as a legacy diagnostic path; both are invoked from the same new task-owned kernel stack switching entry. Existing M5-O through M5-U globals-based entry is untouched and must be migrated separately.

## Limits

Single process, interrupts masked, no process scheduler, no task switch, no concurrent unmap safety, no general ELF runtime, and no syscall services beyond these two narrow experimental FreeWorld calls. This is an incremental task-owned dispatcher proof rather than a production process execution service.

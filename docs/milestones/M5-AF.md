# M5-AF — Task-owned entry cutover

M5-O through M5-U now enter through `__freeworld_m5aa_syscall_entry`. Each proof admits one `TaskSyscallContext`, points `IA32_KERNEL_GS_BASE` at it, and reads the snapshot after `SYSRET`. The global syscall slots and `__freeworld_m5o_syscall_entry` are deleted.

`LSTAR` is programmed once, in `configure_m5o_syscall`. Number 1 uses the task dispatcher's checked add. Number 2 still goes through the existing user-print routine, which records a bad pointer on the task. `FW_SYS_EXIT` is unchanged.

M5-S and M5-T each execute two `SYSCALL` instructions. The task entry increments `seen`, so those proofs now require `seen == 2`. The serial lines are unchanged. M5-O, M5-P, M5-R, and M5-U still see one call.

The M5-X image corrupts the saved return `RCX` in that same entry at compile time, then takes the existing canonical-check `IRETQ`. The page-fault handler reads `abort_seen` from `IA32_KERNEL_GS_BASE`. Its two serial lines are unchanged.

M5-G `INT 0xF2` and its two CR3 slots stay. Those two labels are global so the Windows COFF link can see them.

## Limits

Still one process. Interrupts stay masked. No timer choice and no user scheduler. This does not generalize `copy_from_user` / `copy_to_user`, and it does not teach `start_native` to map an ELF. Number 1 no longer wraps on overflow; an overflow returns `FW_ERR_UNSUPPORTED`, which no current proof exercises. A local compile is not a boot proof. QEMU is not installed here.

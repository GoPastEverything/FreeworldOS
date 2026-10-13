# M5-V — FreeWorld native status ABI v0.1

FreeWorldOS native calls return an **i64 signed result in RAX** on x86-64:

- A nonnegative value is successful. Zero is `FW_OK` for status-only operations; value-returning calls may return a count or other nonnegative value (e.g. print returns 20).
- A negative return is an error. It is the sign-extended 32-bit `FwStatus` from `fw-abi`.
- `FwStatus` uses `i32`; numeric assignments are permanent, never recycled.
- Reserved blocks: general -4096 through -4351, memory -8192 through -8447, handles -12288 through -12543, I/O -16384 through -16639.
- First defined errors: `FW_ERR_NO_SUCH_CALL=-4098`, `FW_ERR_UNSUPPORTED=-4099`, `FW_ERR_USER_ADDRESS=-8193`.
- These are **not Linux errno values**. LinuxFacet will translate FreeWorld statuses to Linux errno, and WinFacet will separately translate to NTSTATUS. Neither is implemented by this milestone.
- Native syscall numbers and status values are experimental until explicit ABI publication. No generic syscall dispatcher or fault-safe user-copy guarantee is implied.

M5-V changes only the existing controlled syscall test path. One process, no timer selection, no scheduler expansion. SMEP/SMAP and SYSRET canonical-return checking are later independent changes.

Additional ABI rules (clarifications after review):
- Native FreeWorld status values never use Linux's raw syscall error range -1 through -4095; the gap is deliberate.
- A negative 64-bit result below i32::MIN is invalid and indicates a broken ABI, never a status.
- A subsystem can receive another assigned block if its first 256 codes are exhausted. Retired codes must never be recycled.

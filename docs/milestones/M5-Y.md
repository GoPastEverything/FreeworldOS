# M5-Y — Unbounded page-span user read walker (initial implementation)

Replaces the M5-U two-page-special-case loop with a general 4-KiB page iterator. For the current process CR3, the read operation validates the entire requested user interval, page by page, before modifying the destination or reading payload. It checks bounds and integer overflow, PRESENT and USER flags at all four levels, and rejects huge-page entries. Copying uses the supervisor physical direct-map alias and never dereferences the original user virtual pointer. Empty output is a no-op.

This change preserves the existing tested M5-R/S/T/U syscall proofs. In particular, the rejected unmapped second page must still copy zero bytes and the successful 8+12 boundary copy must still print hello.

**Review limitations:** This initial slice uses a second page walk during copying; it assumes page mappings remain stable throughout the call. A concurrent unmap or remap is not supported or fault-safe. The existing runtime regression exercises one and two pages; successful three-plus-page mapping tests and copy_to_user writable-permission validation are later acceptance work. This is not yet a production syscall API, multi-process syscall entry, or task scheduler work.

## Additional required QEMU proof

The M5-Y test builds one process root with code U+RX, stack U+RW/NX, and callgate U+RX, backed by distinct frames. The read starts eight bytes before the code-page boundary and copies 8+4096+8 bytes with exact payload verification and counter 4112. A second read starts at the same address but extends through absent fourth page: page preflight fails, the copy counter remains zero and the initialized destination is unchanged. The process drops and returns its seven owned page-table and payload frames. This uses the same walker with an explicitly supplied inactive process root, without scheduling or switching CR3.

# M5-Z — Write to validated user pages (draft)

- Walk all four paging levels for each 4 KiB page of the destination before copying any bytes.
- Demand PRESENT, USER_ACCESSIBLE, and WRITABLE on **every level**, not merely at the leaf; refuse HUGE_PAGE.
- Pass two writes only through the supervisor physical direct-map alias. No STAC needed.
- Reject overflow and upper-half pointers; zero-byte writes succeed without dereferencing.
- Same process-root walker for future active-root integration and inactive-root acceptance proof.
- Tests cover writable leaf success, write spanning from writable stack into RX callgate (no mutation), and a missing later page rejection (no mutation). All prevalidation failures must keep the copied-byte counter at zero.

This remains a **test-only** writer: no production syscall entry or per-task ownership yet. No concurrent unmap/remap safety and no scheduler or timer choice. The current missing-page test overlaps an earlier read-only page; a dedicated writable-writable-missing-page test will be needed before declaring complete acceptance.

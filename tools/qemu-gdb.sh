#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PORT="${FW_GDB_PORT:-1234}"
MEMORY="${FW_QEMU_MEMORY:-256M}"

if [[ "${1:-}" == "--check" ]]; then
  command -v bash >/dev/null
  echo "FreeWorldOS qemu-gdb launcher: syntax/runtime prerequisites deferred"
  exit 0
fi

cargo build

BIOS_IMAGE="$(
  find target -type f -name 'freeworldos-bios.img' -printf '%T@ %p\n'     | sort -nr     | head -n 1     | cut -d' ' -f2-
)"
KERNEL_ELF="$(
  find target -type f -name kernel -path '*x86_64-unknown-none*' -printf '%T@ %p\n'     | sort -nr     | head -n 1     | cut -d' ' -f2-
)"

if [[ -z "$BIOS_IMAGE" || -z "$KERNEL_ELF" ]]; then
  echo "failed to locate BIOS image or kernel ELF after cargo build" >&2
  exit 2
fi

if [[ -n "${GDB:-}" ]]; then
  GDB_BIN="$GDB"
elif command -v gdb-multiarch >/dev/null 2>&1; then
  GDB_BIN="gdb-multiarch"
elif command -v gdb >/dev/null 2>&1; then
  GDB_BIN="gdb"
else
  echo "gdb/gdb-multiarch not found" >&2
  exit 2
fi

SERIAL_LOG="${FW_SERIAL_LOG:-target/freeworld-gdb-serial.log}"
mkdir -p "$(dirname "$SERIAL_LOG")"

qemu-system-x86_64   -drive "format=raw,file=$BIOS_IMAGE"   -serial "file:$SERIAL_LOG"   -display none   -no-reboot   -no-shutdown   -m "$MEMORY"   -S   -gdb "tcp::$PORT" &
QEMU_PID=$!

cleanup() {
  kill "$QEMU_PID" >/dev/null 2>&1 || true
  wait "$QEMU_PID" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

echo "FreeWorldOS QEMU paused before boot."
echo "kernel ELF: $KERNEL_ELF"
echo "serial log: $SERIAL_LOG"
echo "gdb port:   $PORT"
echo
echo "The kernel is relocatable. Use the serial boot line's virt_base with"
echo "tools/symbolize-backtrace.py for panic addresses. For relocated live"
echo "symbols, add-symbol-file with the observed runtime base when needed."

"$GDB_BIN"   -ex "set pagination off"   -ex "target remote :$PORT"   "$KERNEL_ELF"

# FreeWorldOS host tools

These tools run on the development host. They are not FreeWorld-native userland programs and are not part of the kernel trust boundary.

## QEMU + GDB

~~~bash
bash tools/qemu-gdb.sh
~~~

The script builds the current kernel/image, starts QEMU paused with its GDB server enabled, and connects `gdb-multiarch` or `gdb`.

The kernel image is relocatable. Panic logs print the runtime `virt_base`; use that base for relocated symbols when required.

## Panic backtrace symbolizer

~~~bash
python3 tools/symbolize-backtrace.py \
  --kernel target/.../x86_64-unknown-none/.../kernel \
  --log panic-serial.log
~~~

The symbolizer reads `FreeWorldOS: BT[NN]=...` lines and automatically rebases addresses using the `virt_base=...` line from the same serial log.

## Event decoder

~~~bash
python3 tools/events.py --check
python3 tools/events.py --decode 0x0204
python3 tools/events.py --log serial.log
~~~

`tools/events.py` parses the same `kernel/events.def` file the kernel uses to compile event IDs. The host and kernel do not maintain separate event-number tables.

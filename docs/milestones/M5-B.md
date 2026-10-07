# M5-B — User selectors and TSS ring-0 privilege stack

Status: implementation branch after frozen M5-A.

M5-B installs the CPU descriptor and TSS state required before FreeWorld can attempt a controlled CPL3 transition.

It does **not** enter ring 3 and does not program a syscall or native callgate path.

## GDT layout

The bootstrap GDT is now intentionally ordered:

~~~text
index 0  null
index 1  kernel code
index 2  kernel data
index 3  user data   DPL3
index 4  user code   DPL3
index 5  TSS low
index 6  TSS high
~~~

The selectors intended for a future user IRET frame use requestor privilege level 3:

~~~text
user SS = 0x1b
user CS = 0x23
~~~

The GDT entries are created with `Descriptor::user_data_segment()` and
`Descriptor::user_code_segment()`.

The kernel continues running with its existing ring-0 CS and SS. M5-B does not
load the user selectors into active segment registers.

## Selector order and future SYSRET

The user-data descriptor intentionally precedes the user-code descriptor.

That produces the x86-64 ordering required by the fixed SYSRET selector
relationship:

~~~text
SS = base + 8
CS = base + 16
~~~

M5-B only freezes the compatible GDT order. It does not program STAR, LSTAR,
SFMASK, or execute SYSCALL/SYSRET.

## TSS RSP0

The bootstrap CPU TSS now has:

~~~text
privilege_stack_table[0] = top of RING0_PRIVILEGE_STACK
~~~

The stack is:

- 16 KiB;
- 16-byte aligned;
- statically owned by the bootstrap kernel image;
- therefore mapped supervisor-only with the higher-half kernel image;
- distinct from the NMI, double-fault and machine-check IST stacks.

The stack exists for a future CPU privilege transition from CPL3 to CPL0.

## Scheduling limitation

M5-B does not claim that this single bootstrap RSP0 stack is already sufficient
for preemptive user-task scheduling.

If a future interrupt/trap from user mode leaves a saved task frame on this
shared per-CPU stack, the scheduler must not switch away and later allow another
user transition to overwrite that frame.

Before general preemptive user tasks, FreeWorld must choose and prove one of
these designs:

1. update TSS RSP0 to a kernel stack owned by the incoming task before entering
   ring 3; or
2. use the per-CPU entry stack only as a short transition stack and transfer the
   complete saved frame to task-owned kernel storage before a switch is allowed.

M5-B deliberately does not choose that later scheduling policy.

## Architecture API

The x86_64 architecture layer now exposes:

- `user_code_selector()`;
- `user_data_selector()`;
- `ring0_privilege_stack_top()`.

These are production architecture state for later M5 slices, not CI-local
constants.

## M5-B CI proof

The combined self-test image verifies:

- user data descriptor occupies GDT index 3;
- user code descriptor occupies GDT index 4;
- user selectors carry RPL3;
- user code immediately follows user data;
- current CS remains the kernel code selector;
- current SS remains the kernel data selector;
- TSS privilege stack table entry 0 equals the configured RSP0;
- the whole 16 KiB RSP0 stack lies in the higher canonical half;
- RSP0 is 16-byte aligned;
- volatile writes at the bottom and top of the stack round-trip.

Expected markers:

~~~text
FreeWorldOS: M5-B privilege setup: user_ss=0x1b user_cs=0x23 rsp0=0xffff... stack_bytes=16384 gdt_order=kernel_code>kernel_data>user_data>user_code>tss
FreeWorldOS: SELFTEST PASS name=m5b.privilege_setup
FreeWorldOS: M5-B privilege self-test: passed user_code=dpl3 user_data=dpl3 rpl3=ok sysret_order=ok tss_rsp0=higher stack_writable=ok current_cpl=ring0 ring3_entry=off callgate=off
~~~

## Deferred

M5-B does not implement:

- a user address-space object;
- a separate user CR3;
- user stack allocation;
- an IRETQ transition to CPL3;
- user exception return;
- user-task TSS RSP0 switching;
- STAR/LSTAR/SFMASK;
- syscall/trap ABI;
- FwStartInfoV1;
- call table;
- native callgate.

The next slice must build on this loaded GDT/TSS state without smuggling in the
callgate prematurely.

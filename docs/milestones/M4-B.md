# M4-B — Production global VFS namespace

Status: implementation branch. Builds on frozen M4-A native graph and the production M3.5-C scheduler.

M4-B promotes the M4-A graph from a local proof object into the one persistent VFS namespace used by the running kernel.

## Production namespace

Boot-time `vfs::init()` now constructs one `VfsGraph` in global kernel storage and publishes it once initialization is complete.

The graph retains the M4-A identity model:

~~~text
RootHandle
  +
binary-safe native Name segments
  ->
NodeId
~~~

No Windows or Linux path string becomes native identity.

The production namespace exposes kernel-internal operations for:

- root handle / root node identity;
- directory creation;
- file creation;
- file replacement writes;
- copied file reads;
- exact root+segment path resolution.

The global graph persists across task switches and task lifetimes.

## Single-CPU synchronization rule

M4-B remains bootstrap-CPU-only.

Scheduled task access to the global graph uses the scheduler's nested preemption-disable mechanism:

~~~text
task enters VFS
  |
  | preemption_disable_current()
  v
global VfsGraph access
  |
  | IF remains enabled
  | timer interrupts may tick + EOI
  | timer must return to the same task
  v
preemption_enable_current()
~~~

The VFS does not disable interrupts across heap-backed Vec allocation.

No interrupt handler may access the VFS graph in M4-B. Therefore there is no timer/interrupt lock inversion.

This is not an SMP synchronization scheme. Before APs may access the namespace, this storage must move behind a real cross-CPU synchronization design while preserving the no-allocation-in-interrupt-path rule.

## Generic scheduler idle hook

M4-B also adds one generic one-shot idle bootstrap hook to the scheduler.

The hook:

- is installed before scheduler initialization;
- is invoked once from the production idle task after the frozen C2m spawn/lifecycle proof has completed;
- runs with IF enabled on the scheduler-owned idle stack;
- is subsystem-agnostic: the scheduler stores a `fn()` and has no VFS dependency.

The M4-B CI image uses this hook to spawn VFS proof tasks. Later kernel subsystems may use the same mechanism while bootstrap initialization remains single-CPU.

## Cross-task persistence proof

The M4-B CI hook spawns two ordinary kernel tasks through the production `spawn_kernel_task()` API.

Initial queue:

~~~text
slot 0: VFS writer
slot 1: VFS reader
idle:   Running
~~~

The writer creates in the **global production graph**:

~~~text
directory name: Opaque b"shared\xff"
file name:      Opaque b"state\\bin"
payload:        00 46 57 ff 10 20 30
~~~

It publishes the resulting directory and file NodeIds, then waits for the reader.

The timer schedules the reader. The reader independently reconstructs a native Path from the production root handle plus the same two binary-safe segments.

The reader must prove:

- root+segment resolution reaches the writer's exact file NodeId;
- the file's parent is the writer's exact directory NodeId;
- the seven original bytes match exactly.

The reader then replaces the file contents with:

~~~text
00 4d 34 42 fe 99 01
~~~

and exits through the normal scheduler path.

When the writer resumes it reads the global file again and must see the reader's bytes. That proves the namespace is not task-local or a transient test object.

The writer then exits. Idle resumes and reclaims both stopped proof tasks off-stack; CI requires the scheduler to report `stopped_tasks=2`.

Expected proof markers include:

~~~text
FreeWorldOS: M4-B VFS tasks: ... source=spawn_kernel_task
FreeWorldOS: M4-B writer: ... global=production
FreeWorldOS: M4-B reader: ... writer_data=match mutation=committed
FreeWorldOS: M4-B proof: global_namespace=ok cross_task_persistence=ok stable_node_ids=ok binary_bytes=ok reader_mutation_visible=ok scheduler_tasks=ok
FreeWorldOS: scheduler idle reclaimed stopped_tasks=2
~~~

## Scheduler cleanup generalization

C2m originally asserted that the only post-boot deferred object idle could ever reclaim was bootstrap task A.

That is no longer true once subsystems can spawn ordinary kernel tasks.

M4-B preserves the exact C2m first-reclamation proof, then allows later idle cleanup batches to contain arbitrary stopped production tasks. The scheduler reports the number reclaimed but does not reinterpret them as C2m bootstrap objects.

## Deferred to M4-C and later

M4-B still does not implement:

- multiple mounted roots;
- mount table / root projections;
- Linux `/` projection;
- Windows drive / NT namespace projections;
- case-insensitive lookup;
- rename/remove/enumeration;
- permissions/security labels;
- symlinks/hard links;
- persistent filesystem drivers;
- SMP namespace access.

The next slice is the root/mount projection layer over this one native persistent graph.

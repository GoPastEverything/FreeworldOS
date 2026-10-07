use alloc::vec::Vec;
use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

pub mod graph;
pub mod namespace;
pub mod path;

use graph::{NodeId, VfsError, VfsGraph};
use namespace::NamePolicy;
use path::{Name, Path, RootHandle};

struct GlobalVfsStorage(UnsafeCell<MaybeUninit<VfsGraph>>);

// SAFETY: M4-B remains bootstrap-CPU-only. Scheduled-task access is serialized
// by the current task's preemption-disable depth; no interrupt handler accesses
// the VFS graph. SMP must replace this with cross-CPU synchronization.
unsafe impl Sync for GlobalVfsStorage {}

static GLOBAL_VFS: GlobalVfsStorage =
    GlobalVfsStorage(UnsafeCell::new(MaybeUninit::uninit()));
static GLOBAL_VFS_INITIALIZED: AtomicBool = AtomicBool::new(false);

struct GlobalVfsGuard;

impl Deref for GlobalVfsGuard {
    type Target = VfsGraph;

    fn deref(&self) -> &Self::Target {
        // SAFETY: init publishes the graph before GLOBAL_VFS_INITIALIZED.
        // The guard holds the current task non-preemptible on the only CPU.
        unsafe { &*(*GLOBAL_VFS.0.get()).as_ptr() }
    }
}

impl DerefMut for GlobalVfsGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: the scheduler preemption-disable contract gives this task
        // exclusive bootstrap-CPU access until the guard drops.
        unsafe { &mut *(*GLOBAL_VFS.0.get()).as_mut_ptr() }
    }
}

impl Drop for GlobalVfsGuard {
    fn drop(&mut self) {
        crate::rt::scheduler::preemption_enable_current();
    }
}

fn global_graph() -> GlobalVfsGuard {
    assert!(
        GLOBAL_VFS_INITIALIZED.load(Ordering::Acquire),
        "FreeWorld VFS used before initialization"
    );
    assert!(
        crate::arch::interrupts_enabled(),
        "FreeWorld VFS task access requires IF enabled"
    );

    crate::rt::scheduler::preemption_disable_current();
    GlobalVfsGuard
}

pub fn init() {
    assert!(
        !GLOBAL_VFS_INITIALIZED.load(Ordering::Acquire),
        "FreeWorld VFS initialized twice"
    );

    let graph = VfsGraph::new(NamePolicy::CaseSensitive);

    // SAFETY: bootstrap initialization runs once before scheduler publication.
    unsafe {
        (*GLOBAL_VFS.0.get()).write(graph);
    }
    GLOBAL_VFS_INITIALIZED.store(true, Ordering::Release);

    crate::arch::serial::println(
        "  vfs: production native binary-safe graph online root_policy=case-sensitive",
    );
}

pub(crate) fn root_handle() -> RootHandle {
    let graph = global_graph();
    graph.root_handle()
}

pub(crate) fn root_id() -> NodeId {
    let graph = global_graph();
    graph.root_id()
}

pub(crate) fn create_directory(
    parent: NodeId,
    name: Name<'_>,
    policy: NamePolicy,
) -> Result<NodeId, VfsError> {
    let mut graph = global_graph();
    graph.create_directory(parent, name, policy)
}

pub(crate) fn create_file(
    parent: NodeId,
    name: Name<'_>,
) -> Result<NodeId, VfsError> {
    let mut graph = global_graph();
    graph.create_file(parent, name)
}

pub(crate) fn write_file(id: NodeId, bytes: &[u8]) -> Result<(), VfsError> {
    let mut graph = global_graph();
    graph.write_file(id, bytes)
}

pub(crate) fn read_file_copy(id: NodeId) -> Result<Vec<u8>, VfsError> {
    let graph = global_graph();
    Ok(graph.read_file(id)?.to_vec())
}

pub(crate) fn resolve(path: &Path<'_>) -> Result<NodeId, VfsError> {
    let graph = global_graph();
    graph.resolve(path)
}

#[cfg(feature = "m4b-ci-self-test")]
static M4B_WRITER_DONE: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m4b-ci-self-test")]
static M4B_READER_DONE: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m4b-ci-self-test")]
static M4B_DIR_ID: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m4b-ci-self-test")]
static M4B_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "m4b-ci-self-test")]
const M4B_WRITER_BYTES: [u8; 7] = [0x00, 0x46, 0x57, 0xff, 0x10, 0x20, 0x30];
#[cfg(feature = "m4b-ci-self-test")]
const M4B_READER_BYTES: [u8; 7] = [0x00, 0x4d, 0x34, 0x42, 0xfe, 0x99, 0x01];

#[cfg(feature = "m4b-ci-self-test")]
pub(crate) fn install_ci_scheduler_hook() {
    crate::rt::scheduler::install_idle_hook(ci_spawn_vfs_tasks);
}

#[cfg(feature = "m4b-ci-self-test")]
fn ci_spawn_vfs_tasks() {
    assert!(crate::arch::interrupts_enabled());

    // Publish both tasks as one batch so the writer cannot run before the
    // reader exists. spawn_kernel_task nests this preemption-disable depth.
    crate::rt::scheduler::preemption_disable_current();

    let writer = crate::rt::scheduler::spawn_kernel_task(ci_vfs_writer)
        .expect("M4-B failed to spawn VFS writer task");
    let reader = crate::rt::scheduler::spawn_kernel_task(ci_vfs_reader)
        .expect("M4-B failed to spawn VFS reader task");

    assert_eq!(writer.slot, 0);
    assert_eq!(reader.slot, 1);

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M4-B VFS tasks: writer_id={} writer_slot={} reader_id={} reader_slot={} source=spawn_kernel_task\n",
        writer.id,
        writer.slot,
        reader.id,
        reader.slot,
    ));

    crate::rt::scheduler::preemption_enable_current();
}

#[cfg(feature = "m4b-ci-self-test")]
extern "C" fn ci_vfs_writer() -> ! {
    let root = root_id();
    let dir = create_directory(
        root,
        Name::opaque(b"shared\xff"),
        NamePolicy::CaseSensitive,
    )
    .expect("M4-B writer failed to create shared directory");
    let file = create_file(dir, Name::opaque(b"state\\bin"))
        .expect("M4-B writer failed to create shared file");
    write_file(file, &M4B_WRITER_BYTES)
        .expect("M4-B writer failed to write shared file");

    M4B_DIR_ID.store(dir.0, Ordering::Release);
    M4B_FILE_ID.store(file.0, Ordering::Release);
    M4B_WRITER_DONE.store(true, Ordering::Release);

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M4-B writer: created dir={} file={} bytes={} global=production\n",
        dir.0,
        file.0,
        M4B_WRITER_BYTES.len(),
    ));

    while !M4B_READER_DONE.load(Ordering::Acquire) {
        // SAFETY: IF is enabled. The timer may schedule the reader task.
        unsafe {
            core::arch::asm!("hlt", options(nostack, preserves_flags));
        }
    }

    let bytes = read_file_copy(file)
        .expect("M4-B writer could not read reader-updated file");
    assert_eq!(bytes.as_slice(), M4B_READER_BYTES.as_slice());

    crate::arch::serial::println(
        "FreeWorldOS: M4-B proof: global_namespace=ok cross_task_persistence=ok stable_node_ids=ok binary_bytes=ok reader_mutation_visible=ok scheduler_tasks=ok",
    );

    crate::rt::scheduler::exit_current()
}

#[cfg(feature = "m4b-ci-self-test")]
extern "C" fn ci_vfs_reader() -> ! {
    while !M4B_WRITER_DONE.load(Ordering::Acquire) {
        // SAFETY: IF is enabled. The timer may return to the writer task.
        unsafe {
            core::arch::asm!("hlt", options(nostack, preserves_flags));
        }
    }

    let dir_name = Name::opaque(b"shared\xff");
    let file_name = Name::opaque(b"state\\bin");
    let segments = [dir_name, file_name];
    let path = Path {
        root: root_handle(),
        segments: &segments,
    };

    let resolved = resolve(&path)
        .expect("M4-B reader failed to resolve writer-created path");
    assert_eq!(resolved.0, M4B_FILE_ID.load(Ordering::Acquire));

    let dir = graph_parent_for_ci(resolved)
        .expect("M4-B reader failed to read file parent");
    assert_eq!(dir.0, M4B_DIR_ID.load(Ordering::Acquire));

    let bytes = read_file_copy(resolved)
        .expect("M4-B reader failed to read writer-created file");
    assert_eq!(bytes.as_slice(), M4B_WRITER_BYTES.as_slice());

    write_file(resolved, &M4B_READER_BYTES)
        .expect("M4-B reader failed to update shared file");

    M4B_READER_DONE.store(true, Ordering::Release);

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M4-B reader: resolved file={} bytes={} writer_data=match mutation=committed\n",
        resolved.0,
        bytes.len(),
    ));

    crate::rt::scheduler::exit_current()
}

#[cfg(feature = "m4b-ci-self-test")]
fn graph_parent_for_ci(id: NodeId) -> Result<NodeId, VfsError> {
    let graph = global_graph();
    graph.parent(id)?.ok_or(VfsError::NodeNotFound)
}

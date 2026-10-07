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
pub mod projection_path;
pub mod roots;

use graph::{NodeId, VfsError, VfsGraph};
use namespace::NamePolicy;
use path::{Name, Path, RootHandle};
use projection_path::ProjectionPath;
use roots::{ProjectionRoot, RootBindError, RootTable};

struct VfsState {
    graph: VfsGraph,
    roots: RootTable,
}

struct GlobalVfsStorage(UnsafeCell<MaybeUninit<VfsState>>);

// SAFETY: M4-B remains bootstrap-CPU-only. Scheduled-task access is serialized
// by the current task's preemption-disable depth; no interrupt handler accesses
// the VFS graph. SMP must replace this with cross-CPU synchronization.
unsafe impl Sync for GlobalVfsStorage {}

static GLOBAL_VFS: GlobalVfsStorage =
    GlobalVfsStorage(UnsafeCell::new(MaybeUninit::uninit()));
static GLOBAL_VFS_INITIALIZED: AtomicBool = AtomicBool::new(false);

struct GlobalVfsGuard;

impl Deref for GlobalVfsGuard {
    type Target = VfsState;

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
    let roots = RootTable::new(graph.root_handle(), graph.root_id());
    let state = VfsState { graph, roots };

    // SAFETY: bootstrap initialization runs once before scheduler publication.
    unsafe {
        (*GLOBAL_VFS.0.get()).write(state);
    }
    GLOBAL_VFS_INITIALIZED.store(true, Ordering::Release);

    crate::arch::serial::println(
        "  vfs: production native binary-safe graph online root_policy=case-sensitive",
    );
}

pub(crate) fn root_handle() -> RootHandle {
    let state = global_graph();
    state
        .roots
        .handle_for(ProjectionRoot::Native)
        .expect("FreeWorld native VFS root binding disappeared")
}

pub(crate) fn root_id() -> NodeId {
    let state = global_graph();
    state.graph.root_id()
}

pub(crate) fn create_directory(
    parent: NodeId,
    name: Name<'_>,
    policy: NamePolicy,
) -> Result<NodeId, VfsError> {
    let mut graph = global_graph();
    graph.graph.create_directory(parent, name, policy)
}

pub(crate) fn create_file(
    parent: NodeId,
    name: Name<'_>,
) -> Result<NodeId, VfsError> {
    let mut graph = global_graph();
    graph.graph.create_file(parent, name)
}

pub(crate) fn write_file(id: NodeId, bytes: &[u8]) -> Result<(), VfsError> {
    let mut graph = global_graph();
    graph.graph.write_file(id, bytes)
}

pub(crate) fn read_file_copy(id: NodeId) -> Result<Vec<u8>, VfsError> {
    let graph = global_graph();
    Ok(graph.graph.read_file(id)?.to_vec())
}

pub(crate) fn resolve(path: &Path<'_>) -> Result<NodeId, VfsError> {
    let state = global_graph();
    let start = state
        .roots
        .node_for_handle(path.root)
        .ok_or(VfsError::UnknownRoot)?;
    state.graph.resolve_from(start, path.segments)
}

pub(crate) fn resolve_projection_path(
    path: &ProjectionPath,
) -> Result<NodeId, VfsError> {
    let state = global_graph();
    let handle = state
        .roots
        .handle_for(path.root)
        .ok_or(VfsError::UnknownRoot)?;
    let mut current = state
        .roots
        .node_for_handle(handle)
        .ok_or(VfsError::UnknownRoot)?;

    for segment in &path.segments {
        current = state.graph.lookup_child(current, segment.as_name())?;
    }

    Ok(current)
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ProjectionBindError {
    Vfs(VfsError),
    Root(RootBindError),
}

impl From<VfsError> for ProjectionBindError {
    fn from(value: VfsError) -> Self {
        Self::Vfs(value)
    }
}

impl From<RootBindError> for ProjectionBindError {
    fn from(value: RootBindError) -> Self {
        Self::Root(value)
    }
}

pub(crate) fn bind_projection_root(
    node: NodeId,
    projection: ProjectionRoot,
) -> Result<RootHandle, ProjectionBindError> {
    let mut state = global_graph();
    state.graph.node_kind(node)?;
    Ok(state.roots.bind(node, projection)?)
}

pub(crate) fn projection_root(projection: ProjectionRoot) -> Option<RootHandle> {
    let state = global_graph();
    state.roots.handle_for(projection)
}

#[cfg(feature = "m4b-ci-self-test")]
static M4B_WRITER_DONE: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m4b-ci-self-test")]
static M4B_READER_DONE: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m4b-ci-self-test")]
static M4B_DIR_ID: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m4b-ci-self-test")]
static M4B_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "m4c-ci-self-test")]
static M4C_LINUX_ROOT: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m4c-ci-self-test")]
static M4C_WINDOWS_C_ROOT: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "m4e-ci-self-test")]
static M4E_WINDOWS_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "m4b-ci-self-test")]
const M4B_WRITER_BYTES: [u8; 7] = [0x00, 0x46, 0x57, 0xff, 0x10, 0x20, 0x30];
#[cfg(feature = "m4b-ci-self-test")]
const M4B_READER_BYTES: [u8; 7] = [0x00, 0x4d, 0x34, 0x42, 0xfe, 0x99, 0x01];

#[cfg(feature = "m4e-ci-self-test")]
const M4E_WINDOWS_NAME_WTF8: [u8; 11] = [
    b'w', b'i', b'n', b'-', 0xed, 0xa0, 0x80, b'.', b'd', b'a', b't',
];
#[cfg(feature = "m4e-ci-self-test")]
const M4E_WINDOWS_BYTES: [u8; 5] = [0x57, 0x49, 0x4e, 0x00, 0xee];

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

    #[cfg(feature = "m4c-ci-self-test")]
    {
        let linux = bind_projection_root(dir, ProjectionRoot::LinuxRoot)
            .expect("M4-C failed to bind Linux root");
        let windows = bind_projection_root(dir, ProjectionRoot::WindowsDrive(b'C'))
            .expect("M4-C failed to bind Windows C root");

        assert_ne!(linux, windows);
        assert_eq!(
            projection_root(ProjectionRoot::LinuxRoot),
            Some(linux)
        );
        assert_eq!(
            projection_root(ProjectionRoot::WindowsDrive(b'C')),
            Some(windows)
        );
        assert_eq!(
            bind_projection_root(dir, ProjectionRoot::WindowsDrive(b'C')),
            Err(ProjectionBindError::Root(RootBindError::DuplicateProjection))
        );

        M4C_LINUX_ROOT.store(linux.0, Ordering::Release);
        M4C_WINDOWS_C_ROOT.store(windows.0, Ordering::Release);

        crate::arch::serial::write_fmt(format_args!(
            "FreeWorldOS: M4-C roots: native_node={} linux_root={} windows_c_root={} aliases=explicit\n",
            dir.0,
            linux.0,
            windows.0,
        ));
    }

    #[cfg(feature = "m4e-ci-self-test")]
    {
        let windows_file = create_file(dir, Name::wtf8(&M4E_WINDOWS_NAME_WTF8))
            .expect("M4-E failed to create WTF-8 Windows-origin file");
        write_file(windows_file, &M4E_WINDOWS_BYTES)
            .expect("M4-E failed to write Windows-origin file");
        M4E_WINDOWS_FILE_ID.store(windows_file.0, Ordering::Release);

        crate::arch::serial::write_fmt(format_args!(
            "FreeWorldOS: M4-E writer: windows_file={} wtf8_unpaired_surrogate=stored bytes={}\n",
            windows_file.0,
            M4E_WINDOWS_NAME_WTF8.len(),
        ));
    }

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

    #[cfg(feature = "m4c-ci-self-test")]
    {
        let projection_segments = [file_name];

        let linux_path = Path {
            root: RootHandle(M4C_LINUX_ROOT.load(Ordering::Acquire)),
            segments: &projection_segments,
        };
        let windows_path = Path {
            root: RootHandle(M4C_WINDOWS_C_ROOT.load(Ordering::Acquire)),
            segments: &projection_segments,
        };

        let linux_resolved = resolve(&linux_path)
            .expect("M4-C Linux projection failed to resolve shared file");
        let windows_resolved = resolve(&windows_path)
            .expect("M4-C Windows projection failed to resolve shared file");

        assert_eq!(linux_resolved, resolved);
        assert_eq!(windows_resolved, resolved);

        crate::arch::serial::write_fmt(format_args!(
            "FreeWorldOS: M4-C projection proof: linux_root={} windows_c_root={} file_node={} same_native_object=ok explicit_visibility=ok\n",
            linux_path.root.0,
            windows_path.root.0,
            resolved.0,
        ));
    }

    #[cfg(feature = "m4d-ci-self-test")]
    {
        use projection_path::{parse_linux_absolute, LinuxPathError};

        let parsed = parse_linux_absolute(b"/state\\bin")
            .expect("M4-D failed to parse Linux absolute path");
        let parsed_resolved = resolve_projection_path(&parsed)
            .expect("M4-D parsed Linux path failed to resolve");
        assert_eq!(parsed_resolved, resolved);

        let root_only = parse_linux_absolute(b"///")
            .expect("M4-D failed to parse repeated Linux root separators");
        let root_resolved = resolve_projection_path(&root_only)
            .expect("M4-D Linux root-only path failed to resolve");
        assert_eq!(root_resolved.0, M4B_DIR_ID.load(Ordering::Acquire));

        assert_eq!(
            parse_linux_absolute(b"state\\bin"),
            Err(LinuxPathError::NotAbsolute)
        );
        assert_eq!(
            parse_linux_absolute(b"/./state"),
            Err(LinuxPathError::UnsupportedTraversal)
        );
        assert_eq!(
            parse_linux_absolute(b"/bad\0name"),
            Err(LinuxPathError::Nul)
        );

        crate::arch::serial::write_fmt(format_args!(
            "FreeWorldOS: M4-D Linux path proof: input_hex=2f73746174655c62696e file_node={} raw_bytes=ok backslash=data repeated_slash=collapsed dot_traversal=refused\n",
            parsed_resolved.0,
        ));
    }

    #[cfg(feature = "m4e-ci-self-test")]
    {
        use projection_path::{
            parse_windows_drive_absolute,
            WindowsPathError,
        };

        const WINDOWS_PATH: [u16; 14] = [
            b'C' as u16,
            b':' as u16,
            b'\\' as u16,
            b'w' as u16,
            b'i' as u16,
            b'n' as u16,
            b'-' as u16,
            0xd800,
            b'.' as u16,
            b'd' as u16,
            b'a' as u16,
            b't' as u16,
            0, // removed from the slice below; sentinel proves length discipline
            0,
        ];

        let parsed = parse_windows_drive_absolute(&WINDOWS_PATH[..12])
            .expect("M4-E failed to parse Windows DOS absolute path");
        assert_eq!(parsed.root, ProjectionRoot::WindowsDrive(b'C'));

        let parsed_resolved = resolve_projection_path(&parsed)
            .expect("M4-E parsed Windows path failed to resolve");
        assert_eq!(
            parsed_resolved.0,
            M4E_WINDOWS_FILE_ID.load(Ordering::Acquire)
        );

        let windows_bytes = read_file_copy(parsed_resolved)
            .expect("M4-E failed to read Windows-origin file");
        assert_eq!(windows_bytes.as_slice(), M4E_WINDOWS_BYTES.as_slice());

        const LOWERCASE_DRIVE: [u16; 4] = [
            b'c' as u16,
            b':' as u16,
            b'\\' as u16,
            b'/' as u16,
        ];
        let lower = parse_windows_drive_absolute(&LOWERCASE_DRIVE)
            .expect("M4-E failed to normalize lowercase drive");
        assert_eq!(lower.root, ProjectionRoot::WindowsDrive(b'C'));

        const DRIVE_RELATIVE: [u16; 5] = [
            b'C' as u16,
            b':' as u16,
            b'f' as u16,
            b'o' as u16,
            b'o' as u16,
        ];
        assert_eq!(
            parse_windows_drive_absolute(&DRIVE_RELATIVE),
            Err(WindowsPathError::NotDriveAbsolute)
        );

        const DOT_PATH: [u16; 5] = [
            b'C' as u16,
            b':' as u16,
            b'\\' as u16,
            b'.' as u16,
            b'\\' as u16,
        ];
        assert_eq!(
            parse_windows_drive_absolute(&DOT_PATH),
            Err(WindowsPathError::UnsupportedTraversal)
        );

        crate::arch::serial::write_fmt(format_args!(
            "FreeWorldOS: M4-E Windows path proof: drive=C file_node={} utf16=ok wtf8_unpaired_surrogate=ok lowercase_drive=normalized dot_traversal=refused\n",
            parsed_resolved.0,
        ));
    }

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
    graph.graph.parent(id)?.ok_or(VfsError::NodeNotFound)
}

use alloc::vec::Vec;

use super::{
    namespace::NamePolicy,
    path::{Name, NameEncoding, Path, RootHandle},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct NodeId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedName {
    bytes: Vec<u8>,
    encoding: NameEncoding,
}

impl OwnedName {
    pub fn from_name(name: Name<'_>) -> Result<Self, VfsError> {
        validate_segment(name)?;
        Ok(Self {
            bytes: name.bytes.to_vec(),
            encoding: name.encoding,
        })
    }

    pub fn as_name(&self) -> Name<'_> {
        Name {
            bytes: &self.bytes,
            encoding: self.encoding,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    Directory,
    File,
}

#[derive(Debug, Eq, PartialEq)]
pub enum VfsError {
    InvalidName,
    UnknownRoot,
    NodeNotFound,
    NotDirectory,
    NotFile,
    NameExists,
    UnsupportedNamePolicy,
}

enum NodePayload {
    Directory {
        children: Vec<NodeId>,
        policy: NamePolicy,
    },
    File {
        bytes: Vec<u8>,
    },
}

struct Node {
    parent: Option<NodeId>,
    name: Option<OwnedName>,
    payload: NodePayload,
}

impl Node {
    fn kind(&self) -> NodeKind {
        match &self.payload {
            NodePayload::Directory { .. } => NodeKind::Directory,
            NodePayload::File { .. } => NodeKind::File,
        }
    }
}

pub struct VfsGraph {
    nodes: Vec<Node>,
    root: NodeId,
}

impl VfsGraph {
    pub fn new(root_policy: NamePolicy) -> Self {
        let root = NodeId(1);
        let mut nodes = Vec::new();
        nodes.push(Node {
            parent: None,
            name: None,
            payload: NodePayload::Directory {
                children: Vec::new(),
                policy: root_policy,
            },
        });

        Self { nodes, root }
    }

    pub const fn root_handle(&self) -> RootHandle {
        RootHandle(self.root.0)
    }

    pub const fn root_id(&self) -> NodeId {
        self.root
    }

    pub fn node_kind(&self, id: NodeId) -> Result<NodeKind, VfsError> {
        Ok(self.node(id)?.kind())
    }

    pub fn create_directory(
        &mut self,
        parent: NodeId,
        name: Name<'_>,
        policy: NamePolicy,
    ) -> Result<NodeId, VfsError> {
        let owned = OwnedName::from_name(name)?;
        self.ensure_name_policy_supported(policy)?;
        self.ensure_child_name_available(parent, owned.as_name())?;

        let id = self.allocate_node(Node {
            parent: Some(parent),
            name: Some(owned),
            payload: NodePayload::Directory {
                children: Vec::new(),
                policy,
            },
        });

        self.directory_children_mut(parent)?.push(id);
        Ok(id)
    }

    pub fn create_file(
        &mut self,
        parent: NodeId,
        name: Name<'_>,
    ) -> Result<NodeId, VfsError> {
        let owned = OwnedName::from_name(name)?;
        self.ensure_child_name_available(parent, owned.as_name())?;

        let id = self.allocate_node(Node {
            parent: Some(parent),
            name: Some(owned),
            payload: NodePayload::File { bytes: Vec::new() },
        });

        self.directory_children_mut(parent)?.push(id);
        Ok(id)
    }

    pub fn lookup_child(
        &self,
        parent: NodeId,
        name: Name<'_>,
    ) -> Result<NodeId, VfsError> {
        validate_segment(name)?;
        let parent_node = self.node(parent)?;
        let (children, policy) = match &parent_node.payload {
            NodePayload::Directory { children, policy } => (children, *policy),
            NodePayload::File { .. } => return Err(VfsError::NotDirectory),
        };

        self.ensure_name_policy_supported(policy)?;

        for child_id in children {
            let child = self.node(*child_id)?;
            let child_name = child.name.as_ref().ok_or(VfsError::NodeNotFound)?;
            if names_equal(policy, child_name.as_name(), name)? {
                return Ok(*child_id);
            }
        }

        Err(VfsError::NodeNotFound)
    }

    pub fn resolve(&self, path: &Path<'_>) -> Result<NodeId, VfsError> {
        if path.root != self.root_handle() {
            return Err(VfsError::UnknownRoot);
        }

        let mut current = self.root;
        for segment in path.segments {
            current = self.lookup_child(current, *segment)?;
        }
        Ok(current)
    }

    pub fn write_file(&mut self, id: NodeId, bytes: &[u8]) -> Result<(), VfsError> {
        let node = self.node_mut(id)?;
        match &mut node.payload {
            NodePayload::File { bytes: file_bytes } => {
                file_bytes.clear();
                file_bytes.extend_from_slice(bytes);
                Ok(())
            }
            NodePayload::Directory { .. } => Err(VfsError::NotFile),
        }
    }

    pub fn read_file(&self, id: NodeId) -> Result<&[u8], VfsError> {
        let node = self.node(id)?;
        match &node.payload {
            NodePayload::File { bytes } => Ok(bytes),
            NodePayload::Directory { .. } => Err(VfsError::NotFile),
        }
    }

    pub fn parent(&self, id: NodeId) -> Result<Option<NodeId>, VfsError> {
        Ok(self.node(id)?.parent)
    }

    fn allocate_node(&mut self, node: Node) -> NodeId {
        let id = NodeId((self.nodes.len() + 1) as u64);
        self.nodes.push(node);
        id
    }

    fn ensure_child_name_available(
        &self,
        parent: NodeId,
        name: Name<'_>,
    ) -> Result<(), VfsError> {
        match self.lookup_child(parent, name) {
            Ok(_) => Err(VfsError::NameExists),
            Err(VfsError::NodeNotFound) => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn directory_children_mut(
        &mut self,
        id: NodeId,
    ) -> Result<&mut Vec<NodeId>, VfsError> {
        let node = self.node_mut(id)?;
        match &mut node.payload {
            NodePayload::Directory { children, .. } => Ok(children),
            NodePayload::File { .. } => Err(VfsError::NotDirectory),
        }
    }

    fn ensure_name_policy_supported(&self, policy: NamePolicy) -> Result<(), VfsError> {
        match policy {
            NamePolicy::CaseSensitive => Ok(()),
            NamePolicy::CaseInsensitive | NamePolicy::CasePreservingInsensitive => {
                Err(VfsError::UnsupportedNamePolicy)
            }
        }
    }

    fn node(&self, id: NodeId) -> Result<&Node, VfsError> {
        let index = id.0.checked_sub(1).ok_or(VfsError::NodeNotFound)? as usize;
        self.nodes.get(index).ok_or(VfsError::NodeNotFound)
    }

    fn node_mut(&mut self, id: NodeId) -> Result<&mut Node, VfsError> {
        let index = id.0.checked_sub(1).ok_or(VfsError::NodeNotFound)? as usize;
        self.nodes.get_mut(index).ok_or(VfsError::NodeNotFound)
    }
}

fn names_equal(
    policy: NamePolicy,
    left: Name<'_>,
    right: Name<'_>,
) -> Result<bool, VfsError> {
    match policy {
        NamePolicy::CaseSensitive => {
            Ok(left.encoding == right.encoding && left.bytes == right.bytes)
        }
        NamePolicy::CaseInsensitive | NamePolicy::CasePreservingInsensitive => {
            Err(VfsError::UnsupportedNamePolicy)
        }
    }
}

fn validate_segment(name: Name<'_>) -> Result<(), VfsError> {
    if name.bytes.is_empty() || name.bytes.contains(&0) {
        return Err(VfsError::InvalidName);
    }

    match name.encoding {
        NameEncoding::Opaque => {
            if name.bytes.contains(&b'/') {
                return Err(VfsError::InvalidName);
            }
        }
        NameEncoding::Wtf8 => {
            if name.bytes.contains(&b'/') || name.bytes.contains(&b'\\') {
                return Err(VfsError::InvalidName);
            }
        }
    }

    Ok(())
}

#[cfg(feature = "m4a-ci-self-test")]
pub fn ci_self_test() -> Result<(), VfsError> {
    let mut graph = VfsGraph::new(NamePolicy::CaseSensitive);
    let root = graph.root_id();

    let dir_name = Name::opaque(b"bin\xff");
    let file_name = Name::opaque(b"tool\\raw");

    let dir = graph.create_directory(root, dir_name, NamePolicy::CaseSensitive)?;
    let file = graph.create_file(dir, file_name)?;

    let payload = [0x00, 0x46, 0x57, 0xff, 0x7f, 0x10];
    graph.write_file(file, &payload)?;

    let segments = [dir_name, file_name];
    let path = Path {
        root: graph.root_handle(),
        segments: &segments,
    };

    let resolved = graph.resolve(&path)?;
    if resolved != file {
        return Err(VfsError::NodeNotFound);
    }
    if graph.read_file(resolved)? != payload.as_slice() {
        return Err(VfsError::NotFile);
    }
    if graph.parent(file)? != Some(dir) {
        return Err(VfsError::NodeNotFound);
    }
    if graph.node_kind(dir)? != NodeKind::Directory
        || graph.node_kind(file)? != NodeKind::File
    {
        return Err(VfsError::NodeNotFound);
    }

    if graph.create_file(dir, file_name) != Err(VfsError::NameExists) {
        return Err(VfsError::NameExists);
    }

    if OwnedName::from_name(Name::opaque(b"bad/name")) != Err(VfsError::InvalidName) {
        return Err(VfsError::InvalidName);
    }

    if OwnedName::from_name(Name::opaque(b"linux\\backslash")).is_err() {
        return Err(VfsError::InvalidName);
    }

    if OwnedName::from_name(Name::wtf8(b"windows\\separator"))
        != Err(VfsError::InvalidName)
    {
        return Err(VfsError::InvalidName);
    }

    let wrong_root = Path {
        root: RootHandle(graph.root_handle().0 + 99),
        segments: &segments,
    };
    if graph.resolve(&wrong_root) != Err(VfsError::UnknownRoot) {
        return Err(VfsError::UnknownRoot);
    }

    crate::arch::serial::println(
        "FreeWorldOS: M4-A VFS graph self-test: passed binary_names=ok exact_lookup=ok ram_file=ok root_identity=ok separators=projection_only",
    );
    Ok(())
}

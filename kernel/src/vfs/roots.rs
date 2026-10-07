use super::{
    graph::NodeId,
    namespace::NamespaceProjection,
    path::RootHandle,
};

pub const ROOT_BINDING_CAPACITY: usize = 16;
const FIRST_DYNAMIC_ROOT_HANDLE: u64 = 0x1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionRoot {
    Native,
    LinuxRoot,
    WindowsDrive(u8),
}

impl ProjectionRoot {
    pub const fn namespace(self) -> NamespaceProjection {
        match self {
            Self::Native => NamespaceProjection::Native,
            Self::LinuxRoot => NamespaceProjection::Linux,
            Self::WindowsDrive(_) => NamespaceProjection::Windows,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootBinding {
    pub handle: RootHandle,
    pub node: NodeId,
    pub projection: ProjectionRoot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootBindError {
    TableFull,
    DuplicateProjection,
    InvalidWindowsDrive,
}

pub struct RootTable {
    bindings: [Option<RootBinding>; ROOT_BINDING_CAPACITY],
    next_handle: u64,
}

impl RootTable {
    pub fn new(native_handle: RootHandle, native_node: NodeId) -> Self {
        let mut bindings = [None; ROOT_BINDING_CAPACITY];
        bindings[0] = Some(RootBinding {
            handle: native_handle,
            node: native_node,
            projection: ProjectionRoot::Native,
        });

        Self {
            bindings,
            next_handle: FIRST_DYNAMIC_ROOT_HANDLE,
        }
    }

    pub fn bind(
        &mut self,
        node: NodeId,
        projection: ProjectionRoot,
    ) -> Result<RootHandle, RootBindError> {
        validate_projection(projection)?;

        if self.handle_for(projection).is_some() {
            return Err(RootBindError::DuplicateProjection);
        }

        let slot = self
            .bindings
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(RootBindError::TableFull)?;

        let handle = RootHandle(self.next_handle);
        self.next_handle = self
            .next_handle
            .checked_add(1)
            .expect("FreeWorld VFS root handle space exhausted");

        *slot = Some(RootBinding {
            handle,
            node,
            projection,
        });
        Ok(handle)
    }

    pub fn node_for_handle(&self, handle: RootHandle) -> Option<NodeId> {
        self.bindings
            .iter()
            .flatten()
            .find(|binding| binding.handle == handle)
            .map(|binding| binding.node)
    }

    pub fn handle_for(&self, projection: ProjectionRoot) -> Option<RootHandle> {
        self.bindings
            .iter()
            .flatten()
            .find(|binding| binding.projection == projection)
            .map(|binding| binding.handle)
    }

    pub fn binding(&self, handle: RootHandle) -> Option<RootBinding> {
        self.bindings
            .iter()
            .flatten()
            .find(|binding| binding.handle == handle)
            .copied()
    }
}

fn validate_projection(projection: ProjectionRoot) -> Result<(), RootBindError> {
    match projection {
        ProjectionRoot::Native | ProjectionRoot::LinuxRoot => Ok(()),
        ProjectionRoot::WindowsDrive(letter) if letter.is_ascii_uppercase() => Ok(()),
        ProjectionRoot::WindowsDrive(_) => Err(RootBindError::InvalidWindowsDrive),
    }
}


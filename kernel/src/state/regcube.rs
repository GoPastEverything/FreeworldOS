#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    Machine,
    User,
    Session,
    Application,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct NamespaceId(pub u128);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct IdentityId(pub u128);

/// The hot-path key for a RegCube cell.
///
/// Revision is history, not part of the key. Schema is write-time metadata,
/// not part of a normal read lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellAddress<'a> {
    pub namespace: NamespaceId,
    pub identity: IdentityId,
    pub scope: Scope,
    pub property: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct Revision(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct SchemaId(pub u128);

// Architectural invariants:
//
// - RegCube stores state. The VFS stores files.
// - A plain read resolves CellAddress -> current revision directly.
// - History is off the common read path.
// - Schema validation happens on writes/transactions, not every read.
// - Compatibility projections may expose RegCube-backed synthetic files
//   explicitly, but arbitrary filesystem data is never silently absorbed.

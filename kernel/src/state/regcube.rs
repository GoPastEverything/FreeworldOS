#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    Machine,
    User,
    Session,
    Application,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellAddress<'a> {
    pub identity: u128,
    pub scope: Scope,
    pub property: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct Revision(pub u64);

// Architectural invariant:
// RegCube stores state. The VFS stores files.
// Compatibility projections may expose RegCube-backed synthetic files explicitly,
// but arbitrary filesystem data is never silently absorbed into RegCube.

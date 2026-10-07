use alloc::vec::Vec;

use super::{
    graph::{OwnedName, VfsError},
    path::Name,
    roots::ProjectionRoot,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionPath {
    pub root: ProjectionRoot,
    pub segments: Vec<OwnedName>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinuxPathError {
    NotAbsolute,
    Nul,
    UnsupportedTraversal,
    InvalidName,
}

pub fn parse_linux_absolute(raw: &[u8]) -> Result<ProjectionPath, LinuxPathError> {
    if raw.first().copied() != Some(b'/') {
        return Err(LinuxPathError::NotAbsolute);
    }
    if raw.contains(&0) {
        return Err(LinuxPathError::Nul);
    }

    let mut segments = Vec::new();
    for segment in raw[1..].split(|byte| *byte == b'/') {
        if segment.is_empty() {
            continue;
        }

        if segment == b"." || segment == b".." {
            return Err(LinuxPathError::UnsupportedTraversal);
        }

        let owned = OwnedName::from_name(Name::opaque(segment))
            .map_err(|error| match error {
                VfsError::InvalidName => LinuxPathError::InvalidName,
                _ => LinuxPathError::InvalidName,
            })?;
        segments.push(owned);
    }

    Ok(ProjectionPath {
        root: ProjectionRoot::LinuxRoot,
        segments,
    })
}

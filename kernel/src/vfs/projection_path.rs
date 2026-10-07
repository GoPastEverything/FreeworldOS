use alloc::vec::Vec;

use super::{
    graph::OwnedName,
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
            .map_err(|_| LinuxPathError::InvalidName)?;
        segments.push(owned);
    }

    Ok(ProjectionPath {
        root: ProjectionRoot::LinuxRoot,
        segments,
    })
}


#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsPathError {
    NotDriveAbsolute,
    InvalidDrive,
    Nul,
    UnsupportedTraversal,
    UnsupportedColon,
    InvalidName,
}

pub fn parse_windows_drive_absolute(
    units: &[u16],
) -> Result<ProjectionPath, WindowsPathError> {
    if units.contains(&0) {
        return Err(WindowsPathError::Nul);
    }
    if units.len() < 3 || units[1] != b':' as u16 || !is_windows_separator(units[2]) {
        return Err(WindowsPathError::NotDriveAbsolute);
    }

    let drive = normalize_drive_letter(units[0])?;

    let mut segments = Vec::new();
    for segment in units[3..].split(|unit| is_windows_separator(*unit)) {
        if segment.is_empty() {
            continue;
        }

        if segment == [b'.' as u16] || segment == [b'.' as u16, b'.' as u16] {
            return Err(WindowsPathError::UnsupportedTraversal);
        }
        if segment.contains(&(b':' as u16)) {
            return Err(WindowsPathError::UnsupportedColon);
        }

        let bytes = encode_wtf8(segment);
        let owned = OwnedName::from_name(Name::wtf8(&bytes))
            .map_err(|_| WindowsPathError::InvalidName)?;
        segments.push(owned);
    }

    Ok(ProjectionPath {
        root: ProjectionRoot::WindowsDrive(drive),
        segments,
    })
}

fn normalize_drive_letter(unit: u16) -> Result<u8, WindowsPathError> {
    match unit {
        value if value >= b'A' as u16 && value <= b'Z' as u16 => Ok(value as u8),
        value if value >= b'a' as u16 && value <= b'z' as u16 => {
            Ok((value as u8).to_ascii_uppercase())
        }
        _ => Err(WindowsPathError::InvalidDrive),
    }
}

fn is_windows_separator(unit: u16) -> bool {
    unit == b'\\' as u16 || unit == b'/' as u16
}

fn encode_wtf8(units: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut index = 0;

    while index < units.len() {
        let first = units[index];

        if (0xd800..=0xdbff).contains(&first)
            && index + 1 < units.len()
            && (0xdc00..=0xdfff).contains(&units[index + 1])
        {
            let high = (first as u32) - 0xd800;
            let low = (units[index + 1] as u32) - 0xdc00;
            let code_point = 0x1_0000 + (high << 10) + low;
            push_wtf8_code_point(&mut bytes, code_point);
            index += 2;
            continue;
        }

        // Unpaired surrogate code units are intentionally encoded directly as
        // three-byte WTF-8 sequences. Ordinary UTF-8 would reject them.
        push_wtf8_code_point(&mut bytes, first as u32);
        index += 1;
    }

    bytes
}

fn push_wtf8_code_point(bytes: &mut Vec<u8>, code_point: u32) {
    if code_point <= 0x7f {
        bytes.push(code_point as u8);
    } else if code_point <= 0x7ff {
        bytes.push(0xc0 | ((code_point >> 6) as u8));
        bytes.push(0x80 | ((code_point & 0x3f) as u8));
    } else if code_point <= 0xffff {
        bytes.push(0xe0 | ((code_point >> 12) as u8));
        bytes.push(0x80 | (((code_point >> 6) & 0x3f) as u8));
        bytes.push(0x80 | ((code_point & 0x3f) as u8));
    } else {
        bytes.push(0xf0 | ((code_point >> 18) as u8));
        bytes.push(0x80 | (((code_point >> 12) & 0x3f) as u8));
        bytes.push(0x80 | (((code_point >> 6) & 0x3f) as u8));
        bytes.push(0x80 | ((code_point & 0x3f) as u8));
    }
}

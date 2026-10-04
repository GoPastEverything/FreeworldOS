#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct RootHandle(pub u64);

/// How a native FreeWorld name's bytes are interpreted when a compatibility
/// projection needs to reconstruct the original foreign name.
///
/// Opaque is the default for Linux-originated names: Linux filenames are
/// arbitrary bytes except NUL and '/'. Wtf8 is used to round-trip Windows
/// UTF-16 names, including unpaired surrogates, without pretending that every
/// name is valid Unicode text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameEncoding {
    Opaque,
    Wtf8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Name<'a> {
    pub bytes: &'a [u8],
    pub encoding: NameEncoding,
}

impl<'a> Name<'a> {
    pub const fn opaque(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            encoding: NameEncoding::Opaque,
        }
    }

    pub const fn wtf8(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            encoding: NameEncoding::Wtf8,
        }
    }
}

pub struct Path<'a> {
    pub root: RootHandle,
    pub segments: &'a [Name<'a>],
}

// Path syntax is presentation. Object identity is native.
//
// WinFacet may render a root as C:\\..., while LinuxFacet may project a
// foreign root under /volumes/<name>/... (for example /volumes/windows-c/...).
// Native tooling may use tagged forms such as <C:>Games\\... .
//
// Separators are never stored in Name segments.

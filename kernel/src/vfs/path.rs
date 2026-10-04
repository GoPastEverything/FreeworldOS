#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct RootHandle(pub u64);

pub struct Path<'a> {
    pub root: RootHandle,
    pub segments: &'a [&'a str],
}

// Path syntax is presentation. Object identity is native.
// WinFacet may render a root as C:\\..., LinuxFacet as /mnt/...,
// while native tooling may use tagged forms such as <C:>Games\\... .

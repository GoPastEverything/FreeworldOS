#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamePolicy {
    CaseSensitive,
    CaseInsensitive,
    CasePreservingInsensitive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamespaceProjection {
    Native,
    Windows,
    Linux,
}

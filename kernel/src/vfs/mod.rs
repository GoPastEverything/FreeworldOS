pub mod graph;
pub mod namespace;
pub mod path;

pub fn init() {
    crate::arch::serial::println(
        "  vfs: native binary-safe graph model online root_policy=case-sensitive",
    );
}

pub mod namespace;
pub mod path;

pub fn init() {
    crate::arch::serial::println("  vfs: native namespace skeleton online");
}

pub mod handle;
pub mod module;
pub mod process;

pub fn init() {
    crate::arch::serial::println("  object: FW object core placeholder online");
}

pub mod profile;
#[cfg(feature = "m5q-ci-self-test")]
pub mod freeworld_elf;

pub fn init() {
    crate::arch::serial::println("  exec: execution profile model online");
}

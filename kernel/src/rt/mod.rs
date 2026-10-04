pub mod scheduler;
pub mod task;
pub mod time;

pub fn init() {
    scheduler::init();
    crate::arch::serial::println("  rt: deterministic scheduler skeleton online");
}

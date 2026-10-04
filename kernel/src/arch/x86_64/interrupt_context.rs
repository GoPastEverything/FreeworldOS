use core::sync::atomic::{AtomicUsize, Ordering};

static INTERRUPT_DEPTH: AtomicUsize = AtomicUsize::new(0);

pub struct InterruptScope;

pub fn enter() -> InterruptScope {
    INTERRUPT_DEPTH.fetch_add(1, Ordering::AcqRel);
    InterruptScope
}

pub fn in_interrupt() -> bool {
    INTERRUPT_DEPTH.load(Ordering::Acquire) != 0
}

impl Drop for InterruptScope {
    fn drop(&mut self) {
        let previous = INTERRUPT_DEPTH.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous != 0, "interrupt depth underflow");
    }
}

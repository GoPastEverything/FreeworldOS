use core::sync::atomic::{AtomicUsize, Ordering};

static INTERRUPT_DEPTH: AtomicUsize = AtomicUsize::new(0);

pub struct InterruptScope;

/// Marks active interrupt-handler execution on this CPU.
///
/// The depth belongs to handler execution, not to a task's saved context.
/// Before a future timer path hands control to a different task, its scope must
/// be dropped to zero after controller EOI. Resuming a saved interrupt frame
/// through IRETQ does not re-enter this counter.
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

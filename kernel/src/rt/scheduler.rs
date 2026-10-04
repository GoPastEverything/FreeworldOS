use core::sync::atomic::{AtomicU64, Ordering};

static TICK: AtomicU64 = AtomicU64::new(0);

pub fn init() {
    TICK.store(0, Ordering::Release);
}

pub fn tick_count() -> u64 {
    TICK.load(Ordering::Acquire)
}

pub fn on_timer_tick() -> u64 {
    TICK.fetch_add(1, Ordering::AcqRel) + 1
}

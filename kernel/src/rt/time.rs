#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct Tick(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickPeriod {
    pub nanoseconds: u64,
}

pub fn now() -> Tick {
    Tick(crate::arch::timer_ticks())
}

pub fn tick_period() -> Option<TickPeriod> {
    crate::arch::timer_period_ns().map(|nanoseconds| TickPeriod { nanoseconds })
}

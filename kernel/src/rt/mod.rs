pub mod scheduler;
pub mod time;

pub fn init() {
    let ticks = time::now();
    let period = time::tick_period();

    crate::arch::serial::write_fmt(format_args!(
        "  rt: time source online ticks={} period_ns={}\n",
        ticks.0,
        period.map(|value| value.nanoseconds).unwrap_or(0)
    ));
}

use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::arch;

pub const RING_CAPACITY: usize = 256;

pub const LEVEL_TRACE: u8 = 0;
pub const LEVEL_INFO: u8 = 1;
pub const LEVEL_WARN: u8 = 2;
pub const LEVEL_ERROR: u8 = 3;
pub const LEVEL_FATAL: u8 = 4;

pub const SUBSYSTEM_BOOT: u16 = 1;
pub const SUBSYSTEM_SELFTEST: u16 = 2;
pub const SUBSYSTEM_EXCEPTION: u16 = 3;
pub const SUBSYSTEM_MEMORY: u16 = 4;
pub const SUBSYSTEM_DEBUG: u16 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct EventId(pub u32);

macro_rules! fw_event {
    ($name:ident, $id:expr) => {
        pub const $name: EventId = EventId($id);
    };
}

include!("../../events.def");

#[derive(Clone, Copy)]
#[repr(C)]
pub struct EventRecord {
    pub tick: u64,
    pub sequence: u64,
    pub event_id: u32,
    pub cpu: u16,
    pub subsystem: u16,
    pub level: u8,
    pub flags: u8,
    pub reserved: u16,
    pub args: [u64; 4],
}

impl EventRecord {
    const EMPTY: Self = Self {
        tick: 0,
        sequence: 0,
        event_id: 0,
        cpu: 0,
        subsystem: 0,
        level: 0,
        flags: 0,
        reserved: 0,
        args: [0; 4],
    };
}

struct EventSlot {
    published_sequence: AtomicU64,
    record: UnsafeCell<EventRecord>,
}

// SAFETY: M3.5-B is bootstrap-CPU-only. Writers obtain unique monotonically
// increasing sequence numbers. An NMI may interrupt a normal writer, but the
// two writers target different slots unless the ring wraps through all 256
// entries during one nested write, which no current handler can do. A slot is
// published only after its record bytes are complete. SMP must replace this
// with per-CPU rings or equivalent synchronization before APs emit events.
unsafe impl Sync for EventSlot {}

impl EventSlot {
    const fn new() -> Self {
        Self {
            published_sequence: AtomicU64::new(0),
            record: UnsafeCell::new(EventRecord::EMPTY),
        }
    }
}

static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static RING: [EventSlot; RING_CAPACITY] =
    [const { EventSlot::new() }; RING_CAPACITY];

pub fn record(
    event_id: EventId,
    subsystem: u16,
    level: u8,
    args: [u64; 4],
) -> u64 {
    let sequence = NEXT_SEQUENCE
        .fetch_add(1, Ordering::Relaxed)
        .checked_add(1)
        .expect("FreeWorld event sequence exhausted");
    let slot = &RING[(sequence as usize - 1) % RING_CAPACITY];

    let record = EventRecord {
        tick: arch::timer_ticks(),
        sequence,
        event_id: event_id.0,
        cpu: 0,
        subsystem,
        level,
        flags: 0,
        reserved: 0,
        args,
    };

    // SAFETY: See EventSlot::Sync. Publication uses Release after the entire
    // fixed record is written.
    unsafe {
        slot.record.get().write(record);
    }
    slot.published_sequence.store(sequence, Ordering::Release);
    sequence
}

pub fn latest_sequence() -> u64 {
    NEXT_SEQUENCE.load(Ordering::Acquire)
}

pub fn read_sequence(sequence: u64) -> Option<EventRecord> {
    if sequence == 0 {
        return None;
    }

    let slot = &RING[(sequence as usize - 1) % RING_CAPACITY];
    if slot.published_sequence.load(Ordering::Acquire) != sequence {
        return None;
    }

    // SAFETY: Acquire observed the Release publication of this sequence.
    let record = unsafe { slot.record.get().read_volatile() };

    if slot.published_sequence.load(Ordering::Acquire) != sequence {
        return None;
    }

    Some(record)
}

pub fn dump_recent_to_serial(limit: usize) {
    let latest = latest_sequence();
    if latest == 0 || limit == 0 {
        return;
    }

    let bounded = limit.min(RING_CAPACITY);
    let first = latest
        .saturating_sub(bounded as u64)
        .saturating_add(1);

    arch::serial::println("FreeWorldOS: EVENT-RING begin");
    for sequence in first..=latest {
        if let Some(record) = read_sequence(sequence) {
            arch::serial::write_fmt(format_args!(
                "FreeWorldOS: EVENT seq={} tick={} cpu={} subsystem={} level={} id={:#06x} args=[{:#x},{:#x},{:#x},{:#x}]\n",
                record.sequence,
                record.tick,
                record.cpu,
                record.subsystem,
                record.level,
                record.event_id,
                record.args[0],
                record.args[1],
                record.args[2],
                record.args[3],
            ));
        }
    }
    arch::serial::println("FreeWorldOS: EVENT-RING end");
}

#[cfg(feature = "m35b-ci-self-test")]
pub fn ci_self_test() -> Result<(), EventSelfTestError> {
    const ARG0: u64 = 0x4657_4556_454e_5431;
    const ARG1: u64 = 0x4657_4556_454e_5432;

    let sequence = record(
        DEBUG_RING_SELFTEST,
        SUBSYSTEM_DEBUG,
        LEVEL_INFO,
        [ARG0, ARG1, 0, 0],
    );

    let observed = read_sequence(sequence).ok_or(EventSelfTestError::Missing)?;
    if observed.sequence != sequence
        || observed.event_id != DEBUG_RING_SELFTEST.0
        || observed.args[0] != ARG0
        || observed.args[1] != ARG1
    {
        return Err(EventSelfTestError::Mismatch);
    }

    Ok(())
}

#[cfg(feature = "m35b-ci-self-test")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventSelfTestError {
    Missing,
    Mismatch,
}

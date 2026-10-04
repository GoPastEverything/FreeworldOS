use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct ObjectId(pub u64);

static NEXT_OBJECT_ID: AtomicU64 = AtomicU64::new(1);
static LIVE_COUNTER_OBJECTS: AtomicUsize = AtomicUsize::new(0);

pub struct CounterObject {
    id: ObjectId,
    value: AtomicU64,
}

impl CounterObject {
    pub fn new(initial: u64) -> Self {
        let id = ObjectId(NEXT_OBJECT_ID.fetch_add(1, Ordering::Relaxed));
        LIVE_COUNTER_OBJECTS.fetch_add(1, Ordering::AcqRel);

        Self {
            id,
            value: AtomicU64::new(initial),
        }
    }

    pub fn id(&self) -> ObjectId {
        self.id
    }

    pub fn read(&self) -> u64 {
        self.value.load(Ordering::Acquire)
    }

    pub fn increment(&self) -> u64 {
        self.value.fetch_add(1, Ordering::AcqRel) + 1
    }
}

impl Drop for CounterObject {
    fn drop(&mut self) {
        LIVE_COUNTER_OBJECTS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn live_count() -> usize {
    LIVE_COUNTER_OBJECTS.load(Ordering::Acquire)
}

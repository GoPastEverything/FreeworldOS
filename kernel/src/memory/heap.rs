use core::{
    alloc::{GlobalAlloc, Layout},
    cell::UnsafeCell,
    ptr::null_mut,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::memory::{self, MemoryError, PagePermissions, PAGE_SIZE};

pub const HEAP_START: u64 = 0xffff_9000_0000_0000;
pub const HEAP_PAGES: usize = 128;
pub const HEAP_SIZE: usize = HEAP_PAGES * PAGE_SIZE as usize;

const BLOCK_SIZE: usize = 16;
const BLOCK_COUNT: usize = HEAP_SIZE / BLOCK_SIZE;
const BITMAP_WORDS: usize = BLOCK_COUNT / 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeapError {
    Memory(MemoryError),
    AlreadyInitialized,
    SelfTestAllocationFailed,
    SelfTestAccountingMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeapStats {
    pub live_allocations: usize,
    pub live_requested_bytes: usize,
}

struct HeapState {
    initialized: bool,
    used: [u64; BITMAP_WORDS],
}

impl HeapState {
    const fn new() -> Self {
        Self {
            initialized: false,
            used: [0; BITMAP_WORDS],
        }
    }

    fn mark_range(&mut self, start: usize, blocks: usize, allocated: bool) {
        for index in start..start + blocks {
            let word = index / 64;
            let bit = index % 64;
            let mask = 1u64 << bit;

            if allocated {
                self.used[word] |= mask;
            } else {
                self.used[word] &= !mask;
            }
        }
    }

    fn range_is_free(&self, start: usize, blocks: usize) -> bool {
        (start..start + blocks).all(|index| {
            let word = index / 64;
            let bit = index % 64;
            self.used[word] & (1u64 << bit) == 0
        })
    }

    fn allocate(&mut self, layout: Layout) -> *mut u8 {
        if !self.initialized {
            return null_mut();
        }

        let requested = layout.size().max(1);
        let blocks = requested.div_ceil(BLOCK_SIZE);
        if blocks > BLOCK_COUNT {
            return null_mut();
        }

        let alignment = layout.align().max(BLOCK_SIZE);
        for start in 0..=BLOCK_COUNT - blocks {
            let address = HEAP_START as usize + start * BLOCK_SIZE;
            if address % alignment != 0 {
                continue;
            }

            if self.range_is_free(start, blocks) {
                self.mark_range(start, blocks, true);
                return address as *mut u8;
            }
        }

        null_mut()
    }

    fn deallocate(&mut self, pointer: *mut u8, layout: Layout) {
        if pointer.is_null() || !self.initialized {
            return;
        }

        let address = pointer as usize;
        let heap_start = HEAP_START as usize;
        if address < heap_start || address >= heap_start + HEAP_SIZE {
            return;
        }

        let offset = address - heap_start;
        if offset % BLOCK_SIZE != 0 {
            return;
        }

        let requested = layout.size().max(1);
        let blocks = requested.div_ceil(BLOCK_SIZE);
        let start = offset / BLOCK_SIZE;
        if start.checked_add(blocks).is_none_or(|end| end > BLOCK_COUNT) {
            return;
        }

        self.mark_range(start, blocks, false);
    }
}

struct KernelHeap {
    lock: AtomicBool,
    state: UnsafeCell<HeapState>,
}

unsafe impl Sync for KernelHeap {}

impl KernelHeap {
    const fn new() -> Self {
        Self {
            lock: AtomicBool::new(false),
            state: UnsafeCell::new(HeapState::new()),
        }
    }

    fn with_state<R>(&self, operation: impl FnOnce(&mut HeapState) -> R) -> R {
        while self
            .lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }

        // SAFETY: The spin lock serializes all mutable access to state.
        let result = operation(unsafe { &mut *self.state.get() });
        self.lock.store(false, Ordering::Release);
        result
    }

    fn set_initialized(&self) -> Result<(), HeapError> {
        self.with_state(|state| {
            if state.initialized {
                return Err(HeapError::AlreadyInitialized);
            }
            state.initialized = true;
            Ok(())
        })
    }
}

#[global_allocator]
static GLOBAL_HEAP: KernelHeap = KernelHeap::new();

static LIVE_ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static LIVE_REQUESTED_BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = self.with_state(|state| state.allocate(layout));
        if !pointer.is_null() {
            LIVE_ALLOCATIONS.fetch_add(1, Ordering::AcqRel);
            LIVE_REQUESTED_BYTES.fetch_add(layout.size(), Ordering::AcqRel);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        self.with_state(|state| state.deallocate(pointer, layout));
        if !pointer.is_null() {
            LIVE_ALLOCATIONS.fetch_sub(1, Ordering::AcqRel);
            LIVE_REQUESTED_BYTES.fetch_sub(layout.size(), Ordering::AcqRel);
        }
    }
}

pub fn init() -> Result<(), HeapError> {
    for page_index in 0..HEAP_PAGES {
        let frame = memory::allocate_frame().map_err(HeapError::Memory)?;
        let virtual_address = HEAP_START + page_index as u64 * PAGE_SIZE;

        // SAFETY: The M3 heap owns this dedicated virtual range. Each page is
        // mapped exactly once RW+NX to a unique monotonic bootstrap frame.
        unsafe {
            memory::map_page(
                virtual_address,
                frame,
                PagePermissions::read_write(),
            )
        }
        .map_err(HeapError::Memory)?;
    }

    GLOBAL_HEAP.set_initialized()?;

    crate::arch::serial::write_fmt(format_args!(
        "  heap: online base={HEAP_START:#x} size={} KiB pages={} block={} bytes physical_frames=monotonic\n",
        HEAP_SIZE / 1024,
        HEAP_PAGES,
        BLOCK_SIZE,
    ));

    Ok(())
}

pub fn stats() -> HeapStats {
    HeapStats {
        live_allocations: LIVE_ALLOCATIONS.load(Ordering::Acquire),
        live_requested_bytes: LIVE_REQUESTED_BYTES.load(Ordering::Acquire),
    }
}

#[cfg(feature = "m3-ci-self-test")]
pub fn ci_self_test() -> Result<(), HeapError> {
    use alloc::boxed::Box;

    let before = stats();

    let first = Box::new(0x4657_4f53_4d33_4845u64);
    let second = Box::new(0x4150_5f52_4555_5345u64);

    if *first != 0x4657_4f53_4d33_4845 || *second != 0x4150_5f52_4555_5345 {
        return Err(HeapError::SelfTestAllocationFailed);
    }

    let during = stats();
    if during.live_allocations < before.live_allocations + 2 {
        return Err(HeapError::SelfTestAccountingMismatch);
    }

    drop(first);
    drop(second);

    if stats() != before {
        return Err(HeapError::SelfTestAccountingMismatch);
    }

    Ok(())
}

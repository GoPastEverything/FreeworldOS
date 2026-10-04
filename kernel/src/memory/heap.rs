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

const SMALL_POOL_PAGES: usize = 64;
const LARGE_POOL_PAGES: usize = HEAP_PAGES - SMALL_POOL_PAGES;
const LARGE_POOL_START: usize =
    HEAP_START as usize + SMALL_POOL_PAGES * PAGE_SIZE as usize;

const CLASS_COUNT: usize = 7;
const CLASS_SIZES: [usize; CLASS_COUNT] = [32, 64, 128, 256, 512, 1024, 2048];
const CLASS_PAGE_STARTS: [usize; CLASS_COUNT] = [0, 8, 16, 24, 32, 40, 48];
const CLASS_PAGE_COUNTS: [usize; CLASS_COUNT] = [8, 8, 8, 8, 8, 8, 16];

const MIN_CLASS_SIZE: usize = CLASS_SIZES[0];
const SMALL_TRACKED_BLOCKS: usize =
    SMALL_POOL_PAGES * PAGE_SIZE as usize / MIN_CLASS_SIZE;
const SMALL_TRACK_WORDS: usize = SMALL_TRACKED_BLOCKS / 64;

const LARGE_ORDER_COUNT: usize = 7;
const LARGE_MAX_ORDER: usize = LARGE_ORDER_COUNT - 1;
const UNALLOCATED_ORDER: u8 = u8::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeapError {
    Memory(MemoryError),
    AlreadyInitialized,
    SelfTestAllocationFailed,
    SelfTestAccountingMismatch,
    SelfTestAlignmentMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeapStats {
    pub live_allocations: usize,
    pub live_requested_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AllocationKind {
    Small { class: usize },
    Large { order: usize },
}

struct HeapState {
    initialized: bool,
    free_heads: [usize; CLASS_COUNT],
    small_allocated: [u64; SMALL_TRACK_WORDS],
    large_free: [u64; LARGE_ORDER_COUNT],
    large_alloc_order: [u8; LARGE_POOL_PAGES],
}

impl HeapState {
    const fn new() -> Self {
        Self {
            initialized: false,
            free_heads: [0; CLASS_COUNT],
            small_allocated: [0; SMALL_TRACK_WORDS],
            large_free: [0; LARGE_ORDER_COUNT],
            large_alloc_order: [UNALLOCATED_ORDER; LARGE_POOL_PAGES],
        }
    }

    fn initialize(&mut self) {
        for class in 0..CLASS_COUNT {
            let block_size = CLASS_SIZES[class];
            let start = class_start(class);
            let bytes = CLASS_PAGE_COUNTS[class] * PAGE_SIZE as usize;
            let blocks = bytes / block_size;

            let mut head = 0usize;
            for block in (0..blocks).rev() {
                let address = start + block * block_size;
                unsafe { write_next(address, head) };
                head = address;
            }

            self.free_heads[class] = head;
        }

        self.large_free[LARGE_MAX_ORDER] = 1;
        self.initialized = true;
    }

    fn allocate(&mut self, layout: Layout) -> *mut u8 {
        if !self.initialized {
            return null_mut();
        }

        match allocation_kind(layout) {
            Some(AllocationKind::Small { class }) => self.allocate_small(class),
            Some(AllocationKind::Large { order }) => self.allocate_large(order),
            None => null_mut(),
        }
    }

    fn allocate_small(&mut self, class: usize) -> *mut u8 {
        let head = self.free_heads[class];
        if head == 0 {
            return null_mut();
        }

        let next = unsafe { read_next(head) };
        self.free_heads[class] = next;

        let bit = small_tracking_bit(head);
        assert!(
            !self.small_bit_is_set(bit),
            "FreeWorld heap metadata corrupt: small block already allocated"
        );
        self.set_small_bit(bit, true);

        head as *mut u8
    }

    fn allocate_large(&mut self, requested_order: usize) -> *mut u8 {
        let mut source_order = requested_order;
        while source_order <= LARGE_MAX_ORDER && self.large_free[source_order] == 0 {
            source_order += 1;
        }

        if source_order > LARGE_MAX_ORDER {
            return null_mut();
        }

        let source_bit = self.large_free[source_order].trailing_zeros() as usize;
        self.large_free[source_order] &= !(1u64 << source_bit);

        let mut block_index = source_bit;
        while source_order > requested_order {
            source_order -= 1;
            block_index *= 2;
            let right_buddy = block_index + 1;
            self.large_free[source_order] |= 1u64 << right_buddy;
        }

        let start_page = block_index << requested_order;
        assert!(
            self.large_alloc_order[start_page] == UNALLOCATED_ORDER,
            "FreeWorld heap metadata corrupt: large block already allocated"
        );
        self.large_alloc_order[start_page] = requested_order as u8;

        (LARGE_POOL_START + start_page * PAGE_SIZE as usize) as *mut u8
    }

    fn deallocate(&mut self, pointer: *mut u8, layout: Layout) {
        assert!(self.initialized, "FreeWorld heap free before initialization");
        assert!(!pointer.is_null(), "FreeWorld heap invalid null free");

        match allocation_kind(layout) {
            Some(AllocationKind::Small { class }) => {
                self.deallocate_small(pointer as usize, class)
            }
            Some(AllocationKind::Large { order }) => {
                self.deallocate_large(pointer as usize, order)
            }
            None => panic!("FreeWorld heap free with unsupported layout: {layout:?}"),
        }
    }

    fn deallocate_small(&mut self, address: usize, class: usize) {
        let start = class_start(class);
        let end = start + CLASS_PAGE_COUNTS[class] * PAGE_SIZE as usize;
        let block_size = CLASS_SIZES[class];

        assert!(
            address >= start && address < end,
            "FreeWorld heap free outside expected size-class region"
        );
        assert!(
            (address - start) % block_size == 0,
            "FreeWorld heap misaligned size-class free"
        );

        let bit = small_tracking_bit(address);
        assert!(
            self.small_bit_is_set(bit),
            "FreeWorld heap double free or unallocated small block"
        );

        self.set_small_bit(bit, false);
        unsafe { write_next(address, self.free_heads[class]) };
        self.free_heads[class] = address;
    }

    fn deallocate_large(&mut self, address: usize, expected_order: usize) {
        let large_end = LARGE_POOL_START + LARGE_POOL_PAGES * PAGE_SIZE as usize;
        assert!(
            address >= LARGE_POOL_START && address < large_end,
            "FreeWorld heap free outside large-page pool"
        );
        assert!(
            (address - LARGE_POOL_START) % PAGE_SIZE as usize == 0,
            "FreeWorld heap misaligned large-page free"
        );

        let start_page = (address - LARGE_POOL_START) / PAGE_SIZE as usize;
        let block_pages = 1usize << expected_order;
        assert!(
            start_page % block_pages == 0,
            "FreeWorld heap large free not aligned to allocation order"
        );
        assert!(
            self.large_alloc_order[start_page] == expected_order as u8,
            "FreeWorld heap double free, wrong layout, or interior large-page free"
        );

        self.large_alloc_order[start_page] = UNALLOCATED_ORDER;

        let mut order = expected_order;
        let mut block_index = start_page >> order;

        while order < LARGE_MAX_ORDER {
            let buddy = block_index ^ 1;
            let buddy_mask = 1u64 << buddy;

            if self.large_free[order] & buddy_mask == 0 {
                break;
            }

            self.large_free[order] &= !buddy_mask;
            block_index >>= 1;
            order += 1;
        }

        self.large_free[order] |= 1u64 << block_index;
    }

    fn small_bit_is_set(&self, bit: usize) -> bool {
        let word = bit / 64;
        let offset = bit % 64;
        self.small_allocated[word] & (1u64 << offset) != 0
    }

    fn set_small_bit(&mut self, bit: usize, allocated: bool) {
        let word = bit / 64;
        let offset = bit % 64;
        let mask = 1u64 << offset;

        if allocated {
            self.small_allocated[word] |= mask;
        } else {
            self.small_allocated[word] &= !mask;
        }
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

    fn initialize(&self) -> Result<(), HeapError> {
        self.with_state(|state| {
            if state.initialized {
                return Err(HeapError::AlreadyInitialized);
            }
            state.initialize();
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
        debug_assert!(
            !crate::arch::in_interrupt(),
            "FreeWorld heap allocation attempted in interrupt context"
        );

        let pointer = self.with_state(|state| state.allocate(layout));
        if !pointer.is_null() {
            LIVE_ALLOCATIONS.fetch_add(1, Ordering::AcqRel);
            LIVE_REQUESTED_BYTES.fetch_add(layout.size(), Ordering::AcqRel);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        debug_assert!(
            !crate::arch::in_interrupt(),
            "FreeWorld heap deallocation attempted in interrupt context"
        );

        // Invalid or duplicate frees panic inside deallocate before accounting
        // is changed. Kernel heap accounting therefore cannot hide a bad free.
        self.with_state(|state| state.deallocate(pointer, layout));

        LIVE_ALLOCATIONS.fetch_sub(1, Ordering::AcqRel);
        LIVE_REQUESTED_BYTES.fetch_sub(layout.size(), Ordering::AcqRel);
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

    GLOBAL_HEAP.initialize()?;

    crate::arch::serial::write_fmt(format_args!(
        "  heap: online base={HEAP_START:#x} size={} KiB small_pages={} large_pages={} classes=32..2048 large_buddy_orders={} physical_frames=monotonic\n",
        HEAP_SIZE / 1024,
        SMALL_POOL_PAGES,
        LARGE_POOL_PAGES,
        LARGE_ORDER_COUNT,
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
    use alloc::{
        alloc::{alloc, dealloc},
        boxed::Box,
    };

    let before = stats();

    let first = Box::new(0x4657_4f53_4d33_4845u64);
    let second = Box::new(0x4150_5f52_4555_5345u64);

    if *first != 0x4657_4f53_4d33_4845 || *second != 0x4150_5f52_4555_5345 {
        return Err(HeapError::SelfTestAllocationFailed);
    }

    drop(first);
    drop(second);

    const TEST_LAYOUTS: [(usize, usize); 10] = [
        (24, 8),
        (48, 16),
        (100, 32),
        (200, 64),
        (400, 128),
        (800, 256),
        (1600, 512),
        (3000, 16),
        (7000, 64),
        (3000, 4096),
    ];

    for (size, alignment) in TEST_LAYOUTS {
        let layout = Layout::from_size_align(size, alignment)
            .map_err(|_| HeapError::SelfTestAllocationFailed)?;

        let pointer = unsafe { alloc(layout) };
        if pointer.is_null() {
            return Err(HeapError::SelfTestAllocationFailed);
        }
        if pointer as usize % alignment != 0 {
            return Err(HeapError::SelfTestAlignmentMismatch);
        }

        unsafe { dealloc(pointer, layout) };
    }

    if stats() != before {
        return Err(HeapError::SelfTestAccountingMismatch);
    }

    Ok(())
}

fn allocation_kind(layout: Layout) -> Option<AllocationKind> {
    let needed = layout.size().max(1).max(layout.align());

    for (class, size) in CLASS_SIZES.iter().copied().enumerate() {
        if needed <= size {
            return Some(AllocationKind::Small { class });
        }
    }

    let pages = needed.div_ceil(PAGE_SIZE as usize);
    if pages == 0 || pages > LARGE_POOL_PAGES {
        return None;
    }

    let mut order = 0usize;
    let mut capacity = 1usize;
    while capacity < pages {
        capacity <<= 1;
        order += 1;
    }

    (order <= LARGE_MAX_ORDER).then_some(AllocationKind::Large { order })
}

fn class_start(class: usize) -> usize {
    HEAP_START as usize + CLASS_PAGE_STARTS[class] * PAGE_SIZE as usize
}

fn small_tracking_bit(address: usize) -> usize {
    (address - HEAP_START as usize) / MIN_CLASS_SIZE
}

unsafe fn write_next(address: usize, next: usize) {
    unsafe { (address as *mut usize).write(next) };
}

unsafe fn read_next(address: usize) -> usize {
    unsafe { (address as *const usize).read() }
}

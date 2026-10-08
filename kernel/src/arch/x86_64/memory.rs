use core::{
    arch::x86_64::__cpuid,
    cell::UnsafeCell,
    hint::spin_loop,
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, Ordering},
};

use bootloader_api::{
    info::{MemoryRegion, MemoryRegionKind},
    BootInfo,
};
use x86_64::{
    instructions::interrupts,
    registers::{
        control::Cr3,
        model_specific::{Efer, EferFlags},
    },
    structures::paging::{
        mapper::{FlagUpdateError, MapToError, MappedFrame, Translate, TranslateResult, UnmapError},
        FrameAllocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags,
        PhysFrame as X86PhysFrame, Size1GiB, Size2MiB, Size4KiB,
    },
    PhysAddr, VirtAddr,
};

use crate::memory::{
    MemoryCachePolicy, MemoryError, PagePermissions, PhysFrame, PAGE_SIZE,
};

const MIN_ALLOCATABLE_PHYS: u64 = 0x10_0000;
const CPUID_FEATURE_PAT: u32 = 1 << 16;
const IA32_PAT_MSR: u32 = 0x277;
const PAT_STRONG_UNCACHEABLE: u8 = 0x00;

struct ManagerStorage(UnsafeCell<MaybeUninit<X86MemoryManager>>);

// SAFETY: Access to the storage is serialized by MANAGER_LOCK, and M1 keeps
// hardware interrupts disabled. Later SMP work must preserve this exclusion.
unsafe impl Sync for ManagerStorage {}

static MANAGER: ManagerStorage =
    ManagerStorage(UnsafeCell::new(MaybeUninit::uninit()));
static MANAGER_INITIALIZED: AtomicBool = AtomicBool::new(false);
static MANAGER_LOCK: AtomicBool = AtomicBool::new(false);

struct ManagerGuard;

impl ManagerGuard {
    fn acquire() -> Self {
        while MANAGER_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            spin_loop();
        }
        Self
    }
}

impl Drop for ManagerGuard {
    fn drop(&mut self) {
        MANAGER_LOCK.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug)]
struct ReservedRange {
    start: u64,
    end: u64,
}

impl ReservedRange {
    const fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }

    const fn overlaps(self, start: u64, end: u64) -> bool {
        start < self.end && end > self.start
    }
}

#[derive(Clone, Copy, Debug)]
struct FrameStateBitmap {
    physical_start: u64,
    byte_len: usize,
    page_count: usize,
    tracked_frames: u64,
}

impl FrameStateBitmap {
    fn physical_end(self) -> u64 {
        self.physical_start
            .checked_add(self.page_count as u64 * PAGE_SIZE)
            .expect("FreeWorld frame-state bitmap range overflow")
    }

    fn contains_frame(self, frame_start: u64) -> bool {
        frame_start >= self.physical_start && frame_start < self.physical_end()
    }
}

struct BootFrameAllocator {
    regions: &'static [MemoryRegion],
    region_index: usize,
    next_address: u64,
    kernel: ReservedRange,
    ramdisk: Option<ReservedRange>,
    physical_memory_offset: u64,
    recycled_head: Option<u64>,
    recycled_available: usize,
    recycled_total: u64,
    returned_total: u64,
    frame_state: Option<FrameStateBitmap>,
}

impl BootFrameAllocator {
    fn new(
        regions: &'static [MemoryRegion],
        kernel: ReservedRange,
        ramdisk: Option<ReservedRange>,
        physical_memory_offset: u64,
    ) -> Result<Self, MemoryError> {
        let mut allocator = Self {
            regions,
            region_index: 0,
            next_address: MIN_ALLOCATABLE_PHYS,
            kernel,
            ramdisk,
            physical_memory_offset,
            recycled_head: None,
            recycled_available: 0,
            recycled_total: 0,
            returned_total: 0,
            frame_state: None,
        };

        allocator.initialize_frame_state_bitmap()?;
        Ok(allocator)
    }

    fn initialize_frame_state_bitmap(&mut self) -> Result<(), MemoryError> {
        let tracked_end = self
            .regions
            .iter()
            .filter(|region| region.kind == MemoryRegionKind::Usable)
            .map(|region| region.end)
            .max()
            .ok_or(MemoryError::OutOfFrames)?;

        let tracked_frames = tracked_end.div_ceil(PAGE_SIZE);
        let bitmap_bytes_u64 = tracked_frames.div_ceil(8);
        let bitmap_bytes = usize::try_from(bitmap_bytes_u64)
            .map_err(|_| MemoryError::FrameStateBitmapTooLarge)?;
        let bitmap_pages = bitmap_bytes
            .max(1)
            .div_ceil(PAGE_SIZE as usize);

        let physical_start = self
            .reserve_contiguous_fresh_pages(bitmap_pages)
            .ok_or(MemoryError::OutOfFrames)?;

        let bitmap = FrameStateBitmap {
            physical_start,
            byte_len: bitmap_bytes,
            page_count: bitmap_pages,
            tracked_frames,
        };

        let pointer = self.direct_map_pointer(physical_start);
        unsafe {
            core::ptr::write_bytes(
                pointer,
                0,
                bitmap_pages * PAGE_SIZE as usize,
            );
        }

        self.frame_state = Some(bitmap);
        self.mark_bootstrap_consumed_frames();

        Ok(())
    }

    fn mark_bootstrap_consumed_frames(&mut self) {
        for index in 0..self.regions.len() {
            let (is_usable, region_start, region_end) = {
                let region = &self.regions[index];
                (
                    region.kind == MemoryRegionKind::Usable,
                    region.start,
                    region.end,
                )
            };

            if !is_usable || index > self.region_index {
                continue;
            }

            let mut frame_start = align_up(
                region_start.max(MIN_ALLOCATABLE_PHYS),
                PAGE_SIZE,
            );
            let reached_end = if index < self.region_index {
                region_end
            } else {
                self.next_address.min(region_end)
            };

            while frame_start
                .checked_add(PAGE_SIZE)
                .is_some_and(|frame_end| frame_end <= reached_end)
            {
                let frame_end = frame_start + PAGE_SIZE;

                if self.reserved_overlap(frame_start, frame_end).is_none()
                    && !self.is_frame_allocated(frame_start)
                {
                    self.mark_frame_allocated(frame_start);
                }

                frame_start = frame_end;
            }
        }
    }

    fn reserve_contiguous_fresh_pages(&mut self, pages: usize) -> Option<u64> {
        let byte_len = (pages as u64).checked_mul(PAGE_SIZE)?;

        loop {
            let region = self.regions.get(self.region_index)?;

            if region.kind != MemoryRegionKind::Usable {
                self.advance_region();
                continue;
            }

            let region_start = region.start.max(MIN_ALLOCATABLE_PHYS);
            let start = align_up(region_start, PAGE_SIZE);
            if self.next_address < start {
                self.next_address = start;
            }

            let candidate = self.next_address;
            let end = candidate.checked_add(byte_len)?;

            if end > region.end {
                self.advance_region();
                continue;
            }

            if let Some(reserved) = self.reserved_overlap(candidate, end) {
                self.next_address = align_up(reserved.end, PAGE_SIZE);
                continue;
            }

            self.next_address = end;
            return Some(candidate);
        }
    }

    fn next_frame(&mut self) -> Option<X86PhysFrame<Size4KiB>> {
        assert_eq!(
            self.recycled_head.is_some(),
            self.recycled_available != 0,
            "FreeWorld frame recycler corrupt: head/count disagree"
        );

        let frame = if self.recycled_head.is_some() {
            self.pop_recycled_frame()?
        } else {
            self.next_fresh_frame()?
        };

        self.mark_frame_allocated(frame.start_address().as_u64());
        Some(frame)
    }

    fn next_fresh_frame(&mut self) -> Option<X86PhysFrame<Size4KiB>> {
        loop {
            let region = self.regions.get(self.region_index)?;

            if region.kind != MemoryRegionKind::Usable {
                self.advance_region();
                continue;
            }

            let region_start = region.start.max(MIN_ALLOCATABLE_PHYS);
            let start = align_up(region_start, PAGE_SIZE);
            if self.next_address < start {
                self.next_address = start;
            }

            let frame_start = self.next_address;
            let frame_end = frame_start.checked_add(PAGE_SIZE)?;

            if frame_end > region.end {
                self.advance_region();
                continue;
            }

            if let Some(reserved) = self.reserved_overlap(frame_start, frame_end) {
                self.next_address = align_up(reserved.end, PAGE_SIZE);
                continue;
            }

            self.next_address = frame_end;
            return X86PhysFrame::from_start_address(PhysAddr::new(frame_start)).ok();
        }
    }

    fn pop_recycled_frame(&mut self) -> Option<X86PhysFrame<Size4KiB>> {
        let frame_start = self.recycled_head?;

        assert!(
            self.is_managed_frame(frame_start),
            "FreeWorld frame recycler corrupt: head was never allocator-managed"
        );
        assert!(
            !self.is_frame_allocated(frame_start),
            "FreeWorld frame recycler corrupt: free-list head is marked allocated"
        );
        assert!(
            !self.is_allocator_metadata_frame(frame_start),
            "FreeWorld frame recycler corrupt: metadata frame entered free list"
        );

        let pointer = self.direct_map_pointer(frame_start);
        let next = unsafe { (pointer as *const u64).read() };

        if self.recycled_available == 1 {
            assert!(
                next == 0,
                "FreeWorld frame recycler corrupt: final entry has a next link"
            );
        } else {
            assert!(
                next != 0,
                "FreeWorld frame recycler corrupt: list ended before count"
            );
        }

        if next != 0 {
            assert!(
                self.is_managed_frame(next),
                "FreeWorld frame recycler corrupt: next link was never allocator-managed"
            );
            assert!(
                !self.is_frame_allocated(next),
                "FreeWorld frame recycler corrupt: next link points at allocated frame"
            );
            assert!(
                !self.is_allocator_metadata_frame(next),
                "FreeWorld frame recycler corrupt: next link points at metadata"
            );
        }

        self.recycled_head = (next != 0).then_some(next);
        self.recycled_available = self
            .recycled_available
            .checked_sub(1)
            .expect("FreeWorld frame recycler corrupt: count underflow");
        self.recycled_total = self
            .recycled_total
            .checked_add(1)
            .expect("FreeWorld frame recycler reuse counter overflow");

        // Remove free-list metadata and stale contents before the frame is
        // returned to a new owner.
        unsafe {
            core::ptr::write_bytes(pointer, 0, PAGE_SIZE as usize);
        }

        X86PhysFrame::from_start_address(PhysAddr::new(frame_start)).ok()
    }

    fn release_frame(&mut self, frame: PhysFrame) -> Result<(), MemoryError> {
        if frame.start % PAGE_SIZE != 0 {
            return Err(MemoryError::AddressNotAligned);
        }

        if !self.is_managed_frame(frame.start) {
            return Err(MemoryError::FrameNotAllocatorOwned);
        }

        if self.is_allocator_metadata_frame(frame.start) {
            return Err(MemoryError::FrameReservedByAllocator);
        }

        assert!(
            self.is_frame_allocated(frame.start),
            "FreeWorld physical frame double free or invalid return: {:#x}",
            frame.start
        );

        match self.recycled_head {
            Some(head) => {
                assert!(
                    self.recycled_available != 0,
                    "FreeWorld frame recycler corrupt: head present with zero count"
                );
                assert!(
                    self.is_managed_frame(head) && !self.is_frame_allocated(head),
                    "FreeWorld frame recycler corrupt: existing head is not free"
                );
            }
            None => {
                assert!(
                    self.recycled_available == 0,
                    "FreeWorld frame recycler corrupt: count present without head"
                );
            }
        }

        // SAFETY CONTRACT: the caller has surrendered all owner-visible
        // mappings/references to this frame. Scrub before storing the intrusive
        // free-stack link so stale contents cannot cross ownership boundaries.
        let pointer = self.direct_map_pointer(frame.start);
        unsafe {
            core::ptr::write_bytes(pointer, 0, PAGE_SIZE as usize);
            (pointer as *mut u64).write(self.recycled_head.unwrap_or(0));
        }

        self.mark_frame_free(frame.start);
        self.recycled_head = Some(frame.start);
        self.recycled_available = self
            .recycled_available
            .checked_add(1)
            .expect("FreeWorld frame recycler count overflow");
        self.returned_total = self
            .returned_total
            .checked_add(1)
            .expect("FreeWorld frame recycler return counter overflow");
        Ok(())
    }

    fn reuse_stats(&self) -> crate::memory::FrameReuseStats {
        crate::memory::FrameReuseStats {
            available: self.recycled_available,
            returned_total: self.returned_total,
            reused_total: self.recycled_total,
        }
    }

    fn direct_map_pointer(&self, frame_start: u64) -> *mut u8 {
        let virtual_address = self
            .physical_memory_offset
            .checked_add(frame_start)
            .expect("FreeWorld physical direct-map address overflow");

        virtual_address as *mut u8
    }

    fn frame_state(&self) -> FrameStateBitmap {
        self.frame_state
            .expect("FreeWorld frame-state bitmap not initialized")
    }

    fn bitmap_location(&self, frame_start: u64) -> (*mut u8, u8) {
        let state = self.frame_state();
        let frame_index = frame_start / PAGE_SIZE;

        assert!(
            frame_index < state.tracked_frames,
            "FreeWorld frame-state lookup outside tracked physical memory"
        );

        let byte_index = (frame_index / 8) as usize;
        let bit_mask = 1u8 << (frame_index % 8);

        assert!(
            byte_index < state.byte_len,
            "FreeWorld frame-state bitmap byte index overflow"
        );

        let pointer = self.direct_map_pointer(
            state.physical_start + byte_index as u64,
        );

        (pointer, bit_mask)
    }

    fn is_frame_allocated(&self, frame_start: u64) -> bool {
        let (pointer, bit_mask) = self.bitmap_location(frame_start);
        let value = unsafe { pointer.read() };
        value & bit_mask != 0
    }

    fn mark_frame_allocated(&mut self, frame_start: u64) {
        assert!(
            self.is_managed_frame(frame_start),
            "FreeWorld attempted to allocate unmanaged physical frame"
        );
        assert!(
            !self.is_frame_allocated(frame_start),
            "FreeWorld physical frame allocated twice: {frame_start:#x}"
        );

        let (pointer, bit_mask) = self.bitmap_location(frame_start);
        let value = unsafe { pointer.read() };
        unsafe { pointer.write(value | bit_mask) };
    }

    fn mark_frame_free(&mut self, frame_start: u64) {
        assert!(
            self.is_frame_allocated(frame_start),
            "FreeWorld physical frame state clear during free: {frame_start:#x}"
        );

        let (pointer, bit_mask) = self.bitmap_location(frame_start);
        let value = unsafe { pointer.read() };
        unsafe { pointer.write(value & !bit_mask) };
    }

    fn is_allocator_metadata_frame(&self, frame_start: u64) -> bool {
        self.frame_state().contains_frame(frame_start)
    }

    fn is_managed_frame(&self, frame_start: u64) -> bool {
        let frame_end = match frame_start.checked_add(PAGE_SIZE) {
            Some(end) => end,
            None => return false,
        };

        if frame_start < MIN_ALLOCATABLE_PHYS
            || frame_start % PAGE_SIZE != 0
            || self.reserved_overlap(frame_start, frame_end).is_some()
        {
            return false;
        }

        let Some((index, _region)) = self
            .regions
            .iter()
            .enumerate()
            .find(|(_, region)| {
                region.kind == MemoryRegionKind::Usable
                    && frame_start >= region.start.max(MIN_ALLOCATABLE_PHYS)
                    && frame_end <= region.end
            })
        else {
            return false;
        };

        if index < self.region_index {
            return true;
        }

        index == self.region_index && frame_end <= self.next_address
    }

    fn advance_region(&mut self) {
        self.region_index += 1;
        self.next_address = MIN_ALLOCATABLE_PHYS;
    }

    fn reserved_overlap(&self, start: u64, end: u64) -> Option<ReservedRange> {
        if self.kernel.overlaps(start, end) {
            return Some(self.kernel);
        }

        self.ramdisk
            .filter(|reserved| reserved.overlaps(start, end))
    }
}

// SAFETY: fresh frames come only from bootloader regions marked Usable, with
// low memory and explicit kernel/ramdisk reservations excluded. The always-on
// frame-state bitmap enforces a clear->allocated transition for every frame
// handed out and allocated->clear before any frame enters the recycler.
unsafe impl FrameAllocator<Size4KiB> for BootFrameAllocator {
    fn allocate_frame(&mut self) -> Option<X86PhysFrame<Size4KiB>> {
        self.next_frame()
    }
}

struct X86MemoryManager {
    mapper: OffsetPageTable<'static>,
    allocator: BootFrameAllocator,
    physical_memory_offset: u64,
}

pub fn init(boot_info: &'static mut BootInfo) -> Result<(), MemoryError> {
    interrupts::without_interrupts(|| {
        let _guard = ManagerGuard::acquire();

        if MANAGER_INITIALIZED.load(Ordering::Acquire) {
            return Err(MemoryError::AlreadyInitialized);
        }

        let physical_memory_offset = boot_info
            .physical_memory_offset
            .into_option()
            .ok_or(MemoryError::NoPhysicalMemoryMapping)?;

        crate::memory::address_space::validate_boot_layout(
            boot_info,
            physical_memory_offset,
        )?;

        log_boot_memory(boot_info, physical_memory_offset);

        // Enable NX so PageTableFlags::NO_EXECUTE is actually enforced.
        // SAFETY: Enabling NXE in long mode is a monotonic hardening change.
        unsafe {
            Efer::update(|flags| flags.insert(EferFlags::NO_EXECUTE_ENABLE));
        }

        let level_4_table = unsafe {
            active_level_4_table(VirtAddr::new(physical_memory_offset))
        };
        let mapper = unsafe {
            OffsetPageTable::new(level_4_table, VirtAddr::new(physical_memory_offset))
        };

        let kernel = ReservedRange::new(
            boot_info.kernel_addr,
            boot_info.kernel_addr.saturating_add(boot_info.kernel_len),
        );
        let ramdisk = boot_info
            .ramdisk_addr
            .into_option()
            .filter(|_| boot_info.ramdisk_len != 0)
            .map(|start| {
                ReservedRange::new(
                    start,
                    start.saturating_add(boot_info.ramdisk_len),
                )
            });

        let regions: &'static [MemoryRegion] = &boot_info.memory_regions;
        let allocator =
            BootFrameAllocator::new(regions, kernel, ramdisk, physical_memory_offset)?;

        let bitmap = allocator.frame_state();
        serial_memory_line(format_args!(
            "  memory: frame-state bitmap phys=[{:#x}..{:#x}) bytes={} pages={} tracked_frames={}\n",
            bitmap.physical_start,
            bitmap.physical_end(),
            bitmap.byte_len,
            bitmap.page_count,
            bitmap.tracked_frames,
        ));

        let manager = X86MemoryManager {
            mapper,
            allocator,
            physical_memory_offset,
        };

        // SAFETY: The storage is written exactly once while the manager lock is
        // held, before MANAGER_INITIALIZED becomes visible.
        unsafe {
            (*MANAGER.0.get()).write(manager);
        }
        MANAGER_INITIALIZED.store(true, Ordering::Release);

        let (level_4_frame, _) = Cr3::read();
        serial_memory_line(format_args!(
            "  memory: active CR3={:#x} page_size={} bytes NX=on\n",
            level_4_frame.start_address().as_u64(),
            PAGE_SIZE
        ));
        serial_memory_line(format_args!(
            "  memory: allocator floor={MIN_ALLOCATABLE_PHYS:#x} kernel/ramdisk excluded\n"
        ));

        Ok(())
    })
}

pub fn allocate_frame() -> Result<PhysFrame, MemoryError> {
    assert!(
        !crate::debug::panic::is_active(),
        "FreeWorld frame allocation attempted during panic/fatal dump"
    );

    with_manager(|manager| {
        manager
            .allocator
            .next_frame()
            .map(|frame| PhysFrame {
                start: frame.start_address().as_u64(),
            })
            .ok_or(MemoryError::OutOfFrames)
    })?
}

pub unsafe fn free_frame(frame: PhysFrame) -> Result<(), MemoryError> {
    assert!(
        !crate::debug::panic::is_active(),
        "FreeWorld frame return attempted during panic/fatal dump"
    );

    with_manager(|manager| manager.allocator.release_frame(frame))?
}

pub fn frame_reuse_stats() -> Result<crate::memory::FrameReuseStats, MemoryError> {
    with_manager(|manager| manager.allocator.reuse_stats())
}


#[cfg(feature = "m5f-ci-self-test")]
mod process_cr3_probe;
mod process_leaf;
pub use process_leaf::{
    InactiveUserLeaf, InactiveUserLeafInfo,
    destroy_inactive_user_leaf, inspect_inactive_user_leaf, map_one_inactive_user_leaf,
};
#[cfg(feature = "m5e-ci-self-test")]
pub use process_leaf::ci_probe_inactive_user_leaf;
#[cfg(feature = "m5f-ci-self-test")]
pub use process_leaf::{ControlledCr3Proof, ci_controlled_cr3_roundtrip};

const PML4_ENTRY_COUNT: usize = 512;
const PML4_LOWER_HALF_ENTRIES: usize = 256;

pub fn create_process_address_space_root() -> Result<PhysFrame, MemoryError> {
    assert!(
        !crate::debug::panic::is_active(),
        "FreeWorld process address-space allocation attempted during panic/fatal dump"
    );

    with_manager(|manager| {
        let root = manager
            .allocator
            .next_frame()
            .map(|frame| PhysFrame {
                start: frame.start_address().as_u64(),
            })
            .ok_or(MemoryError::OutOfFrames)?;

        let root_pointer = manager.allocator.direct_map_pointer(root.start);
        unsafe {
            core::ptr::write_bytes(root_pointer, 0, PAGE_SIZE as usize);
        }

        let (kernel_root_frame, _) = Cr3::read();
        let kernel_pointer = manager
            .allocator
            .direct_map_pointer(kernel_root_frame.start_address().as_u64())
            as *const PageTable;
        let process_pointer = root_pointer as *mut PageTable;

        let kernel_root = unsafe { &*kernel_pointer };
        let process_root = unsafe { &mut *process_pointer };

        // Lower-half entries stay explicitly zero even if the active kernel
        // PML4 retains empty intermediate tables from earlier temporary user
        // mappings. Only the shared higher-half kernel entries are copied.
        for index in PML4_LOWER_HALF_ENTRIES..PML4_ENTRY_COUNT {
            if kernel_root[index].is_unused() {
                process_root[index].set_unused();
            } else {
                process_root[index].set_addr(
                    kernel_root[index].addr(),
                    kernel_root[index].flags(),
                );
            }
        }

        Ok(root)
    })?
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessAddressSpaceRootInfo {
    pub root_frame: PhysFrame,
    pub active_kernel_root_frame: PhysFrame,
    pub lower_half_empty: bool,
    pub higher_half_matches_kernel: bool,
    pub higher_half_user_accessible: bool,
}

pub fn inspect_process_address_space_root(
    root: PhysFrame,
) -> Result<ProcessAddressSpaceRootInfo, MemoryError> {
    with_manager(|manager| {
        if !manager.allocator.is_managed_frame(root.start)
            || !manager.allocator.is_frame_allocated(root.start)
        {
            return Err(MemoryError::InvalidAddressSpaceRoot);
        }

        let (kernel_root_frame, _) = Cr3::read();
        let kernel_phys = kernel_root_frame.start_address().as_u64();

        let process_pointer =
            manager.allocator.direct_map_pointer(root.start) as *const PageTable;
        let kernel_pointer =
            manager.allocator.direct_map_pointer(kernel_phys) as *const PageTable;

        let process_root = unsafe { &*process_pointer };
        let kernel_root = unsafe { &*kernel_pointer };

        let lower_half_empty = (0..PML4_LOWER_HALF_ENTRIES)
            .all(|index| process_root[index].is_unused());

        let mut higher_half_matches_kernel = true;
        let mut higher_half_user_accessible = false;

        for index in PML4_LOWER_HALF_ENTRIES..PML4_ENTRY_COUNT {
            let process_entry = &process_root[index];
            let kernel_entry = &kernel_root[index];

            if process_entry.addr() != kernel_entry.addr()
                || process_entry.flags() != kernel_entry.flags()
            {
                higher_half_matches_kernel = false;
            }

            if process_entry
                .flags()
                .contains(PageTableFlags::USER_ACCESSIBLE)
            {
                higher_half_user_accessible = true;
            }
        }

        Ok(ProcessAddressSpaceRootInfo {
            root_frame: root,
            active_kernel_root_frame: PhysFrame {
                start: kernel_phys,
            },
            lower_half_empty,
            higher_half_matches_kernel,
            higher_half_user_accessible,
        })
    })?
}

pub unsafe fn destroy_process_address_space_root(
    root: PhysFrame,
) -> Result<(), MemoryError> {
    assert!(
        !crate::debug::panic::is_active(),
        "FreeWorld process address-space destruction attempted during panic/fatal dump"
    );

    with_manager(|manager| {
        let (active_root, _) = Cr3::read();
        if active_root.start_address().as_u64() == root.start {
            return Err(MemoryError::AddressSpaceRootActive);
        }

        if !manager.allocator.is_managed_frame(root.start)
            || !manager.allocator.is_frame_allocated(root.start)
        {
            return Err(MemoryError::InvalidAddressSpaceRoot);
        }

        let process_pointer =
            manager.allocator.direct_map_pointer(root.start) as *const PageTable;
        let process_root = unsafe { &*process_pointer };

        if (0..PML4_LOWER_HALF_ENTRIES)
            .any(|index| !process_root[index].is_unused())
        {
            return Err(MemoryError::AddressSpaceRootNotEmpty);
        }

        manager.allocator.release_frame(root)
    })?
}

#[cfg(feature = "m35a-ci-self-test")]
pub fn frame_is_allocated_for_test(frame: PhysFrame) -> Result<bool, MemoryError> {
    with_manager(|manager| {
        if !manager.allocator.is_managed_frame(frame.start) {
            return Err(MemoryError::FrameNotAllocatorOwned);
        }

        Ok(manager.allocator.is_frame_allocated(frame.start))
    })?
}

pub unsafe fn map_page(
    virtual_address: u64,
    frame: PhysFrame,
    permissions: PagePermissions,
) -> Result<(), MemoryError> {
    if permissions.writable() && permissions.executable() {
        return Err(MemoryError::WriteExecuteDenied);
    }

    with_manager(|manager| {
        let virt = VirtAddr::try_new(virtual_address)
            .map_err(|_| MemoryError::InvalidVirtualAddress)?;
        let page = Page::<Size4KiB>::from_start_address(virt)
            .map_err(|_| MemoryError::AddressNotAligned)?;
        let physical = X86PhysFrame::<Size4KiB>::from_start_address(
            PhysAddr::try_new(frame.start)
                .map_err(|_| MemoryError::InvalidPhysicalFrame)?,
        )
        .map_err(|_| MemoryError::AddressNotAligned)?;

        let mut flags = PageTableFlags::PRESENT;
        if permissions.writable() {
            flags |= PageTableFlags::WRITABLE;
        }
        if permissions.user() {
            flags |= PageTableFlags::USER_ACCESSIBLE;
        }
        if !permissions.executable() {
            flags |= PageTableFlags::NO_EXECUTE;
        }
        if matches!(permissions.cache_policy(), MemoryCachePolicy::Device) {
            validate_device_cache_policy()?;
            // x86 PAT index 3 (PWT=1, PCD=1, PAT=0) has been verified as
            // StrongUncacheable above.
            flags |= PageTableFlags::WRITE_THROUGH | PageTableFlags::NO_CACHE;
        }

        let result = unsafe {
            manager
                .mapper
                .map_to(page, physical, flags, &mut manager.allocator)
        };

        match result {
            Ok(flush) => {
                flush.flush();
                Ok(())
            }
            Err(MapToError::FrameAllocationFailed) => Err(MemoryError::OutOfFrames),
            Err(MapToError::ParentEntryHugePage) => Err(MemoryError::ParentHugePage),
            Err(MapToError::PageAlreadyMapped(_)) => Err(MemoryError::PageAlreadyMapped),
        }
    })?
}


pub fn harden_direct_map_device_alias(
    physical_address: u64,
) -> Result<bool, MemoryError> {
    validate_device_cache_policy()?;

    with_manager(|manager| {
        let virtual_address = manager
            .physical_memory_offset
            .checked_add(physical_address)
            .ok_or(MemoryError::InvalidVirtualAddress)?;
        let virtual_address = VirtAddr::try_new(virtual_address)
            .map_err(|_| MemoryError::InvalidVirtualAddress)?;

        match manager.mapper.translate(virtual_address) {
            TranslateResult::NotMapped => Ok(false),
            TranslateResult::InvalidFrameAddress(_) => {
                Err(MemoryError::InvalidFrameAddress)
            }
            TranslateResult::Mapped { frame, flags, .. } => {
                let device_flags = flags
                    | PageTableFlags::NO_EXECUTE
                    | PageTableFlags::WRITE_THROUGH
                    | PageTableFlags::NO_CACHE;

                match frame {
                    MappedFrame::Size4KiB(_) => {
                        let page = Page::<Size4KiB>::containing_address(virtual_address);
                        match unsafe { manager.mapper.update_flags(page, device_flags) } {
                            Ok(flush) => {
                                flush.flush();
                                Ok(true)
                            }
                            Err(FlagUpdateError::PageNotMapped) => Ok(false),
                            Err(FlagUpdateError::ParentEntryHugePage) => {
                                Err(MemoryError::DirectMapUnexpectedPageSize)
                            }
                        }
                    }
                    MappedFrame::Size2MiB(_) => {
                        let page = Page::<Size2MiB>::containing_address(virtual_address);
                        match unsafe { manager.mapper.update_flags(page, device_flags) } {
                            Ok(flush) => {
                                flush.flush();
                                Ok(true)
                            }
                            Err(FlagUpdateError::PageNotMapped) => Ok(false),
                            Err(FlagUpdateError::ParentEntryHugePage) => {
                                Err(MemoryError::DirectMapUnexpectedPageSize)
                            }
                        }
                    }
                    MappedFrame::Size1GiB(_) => {
                        let _page = Page::<Size1GiB>::containing_address(virtual_address);
                        Err(MemoryError::DirectMapUnexpectedPageSize)
                    }
                }
            }
        }
    })?
}

pub fn unmap_page(virtual_address: u64) -> Result<PhysFrame, MemoryError> {
    with_manager(|manager| {
        let virt = VirtAddr::try_new(virtual_address)
            .map_err(|_| MemoryError::InvalidVirtualAddress)?;
        let page = Page::<Size4KiB>::from_start_address(virt)
            .map_err(|_| MemoryError::AddressNotAligned)?;

        match manager.mapper.unmap(page) {
            Ok((frame, flush)) => {
                flush.flush();
                Ok(PhysFrame {
                    start: frame.start_address().as_u64(),
                })
            }
            Err(UnmapError::PageNotMapped) => Err(MemoryError::PageNotMapped),
            Err(UnmapError::ParentEntryHugePage) => Err(MemoryError::ParentHugePage),
            Err(UnmapError::InvalidFrameAddress(_)) => {
                Err(MemoryError::InvalidFrameAddress)
            }
        }
    })?
}


fn validate_device_cache_policy() -> Result<(), MemoryError> {
    let features = __cpuid(1);
    if features.edx & CPUID_FEATURE_PAT == 0 {
        return Err(MemoryError::PageAttributeTableUnsupported);
    }

    let pat = unsafe { read_msr(IA32_PAT_MSR) };
    let entry3 = ((pat >> (3 * 8)) & 0xff) as u8;
    if entry3 != PAT_STRONG_UNCACHEABLE {
        return Err(MemoryError::DeviceCachePolicyUnavailable);
    }

    Ok(())
}

unsafe fn read_msr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;

    // SAFETY: Caller verifies architectural support for the requested MSR.
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags)
        );
    }

    (u64::from(high) << 32) | u64::from(low)
}

fn with_manager<R>(
    operation: impl FnOnce(&mut X86MemoryManager) -> R,
) -> Result<R, MemoryError> {
    interrupts::without_interrupts(|| {
        let _guard = ManagerGuard::acquire();

        if !MANAGER_INITIALIZED.load(Ordering::Acquire) {
            return Err(MemoryError::NotInitialized);
        }

        // SAFETY: Initialization has completed and access is serialized by the
        // manager lock for the full duration of the mutable borrow.
        let manager = unsafe { &mut *(*MANAGER.0.get()).as_mut_ptr() };
        Ok(operation(manager))
    })
}

unsafe fn active_level_4_table(
    physical_memory_offset: VirtAddr,
) -> &'static mut PageTable {
    let (level_4_frame, _) = Cr3::read();
    let physical = level_4_frame.start_address();
    let virtual_address = physical_memory_offset + physical.as_u64();
    let page_table_ptr: *mut PageTable = virtual_address.as_mut_ptr();

    // SAFETY: The bootloader maps all physical memory at the provided offset,
    // and CR3 identifies the active level-4 page table frame. This function is
    // called once while establishing exclusive FreeWorld page-table ownership.
    unsafe { &mut *page_table_ptr }
}

fn log_boot_memory(boot_info: &BootInfo, physical_memory_offset: u64) {
    let mut usable_total = 0u64;

    serial_memory_line(format_args!(
        "  memory: kernel phys=[{:#x}..{:#x}) virt_base={:#x}\n",
        boot_info.kernel_addr,
        boot_info.kernel_addr.saturating_add(boot_info.kernel_len),
        boot_info.kernel_image_offset
    ));
    if let Some(ramdisk_start) = boot_info.ramdisk_addr.into_option() {
        if boot_info.ramdisk_len != 0 {
            serial_memory_line(format_args!(
                "  memory: ramdisk phys=[{:#x}..{:#x})\n",
                ramdisk_start,
                ramdisk_start.saturating_add(boot_info.ramdisk_len)
            ));
        }
    }
    serial_memory_line(format_args!(
        "  memory: physical map offset={physical_memory_offset:#x}\n"
    ));

    serial_memory_line(format_args!(
        "  memory: address-space split user=[{:#x}..{:#x}) kernel_start={:#x} kernel_image={:#x} kernel_stack={:#x} boot_info={:#x} phys_map={:#x}\n",
        crate::memory::address_space::USER_ADDRESS_START,
        crate::memory::address_space::USER_ADDRESS_END_EXCLUSIVE,
        crate::memory::address_space::KERNEL_ADDRESS_START,
        boot_info.kernel_image_offset,
        boot_info.kernel_stack_bottom,
        boot_info as *const BootInfo as u64,
        physical_memory_offset,
    ));

    for region in boot_info.memory_regions.iter() {
        if region.kind == MemoryRegionKind::Usable {
            let length = region.end.saturating_sub(region.start);
            usable_total = usable_total.saturating_add(length);
            serial_memory_line(format_args!(
                "  memory: usable [{:#x}..{:#x}) {} KiB\n",
                region.start,
                region.end,
                length / 1024
            ));
        }
    }

    serial_memory_line(format_args!(
        "  memory: usable total={} KiB\n",
        usable_total / 1024
    ));
}

fn serial_memory_line(args: core::fmt::Arguments<'_>) {
    super::serial::write_fmt(args);
}

const fn align_up(value: u64, alignment: u64) -> u64 {
    let mask = alignment - 1;
    value.saturating_add(mask) & !mask
}

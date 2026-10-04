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

struct BootFrameAllocator {
    regions: &'static [MemoryRegion],
    region_index: usize,
    next_address: u64,
    kernel: ReservedRange,
    ramdisk: Option<ReservedRange>,
}

impl BootFrameAllocator {
    fn new(
        regions: &'static [MemoryRegion],
        kernel: ReservedRange,
        ramdisk: Option<ReservedRange>,
    ) -> Self {
        Self {
            regions,
            region_index: 0,
            next_address: MIN_ALLOCATABLE_PHYS,
            kernel,
            ramdisk,
        }
    }

    fn next_frame(&mut self) -> Option<X86PhysFrame<Size4KiB>> {
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

// SAFETY: next_frame walks only regions marked Usable by bootloader_api,
// excludes low memory and explicit kernel/ramdisk reservations, and advances
// monotonically, so it never returns the same frame twice.
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
        let allocator = BootFrameAllocator::new(regions, kernel, ramdisk);

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

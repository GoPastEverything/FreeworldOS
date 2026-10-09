use super::*;

// M5-E owns exactly one leaf and three private lower-half paging structures.
// The root is never loaded into CR3. This intentionally is not a generic VM API.
#[derive(Debug)]
pub struct InactiveUserLeaf {
    virtual_address: u64,
    pub data_frame: PhysFrame,
    l3: PhysFrame,
    l2: PhysFrame,
    l1: PhysFrame,
    leaf_flags: PageTableFlags,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InactiveUserLeafInfo {
    pub root_frame: PhysFrame,
    pub virtual_address: u64,
    pub data_frame: PhysFrame,
    pub ancestor_user_accessible: bool,
    pub leaf_user_accessible: bool,
    pub leaf_writable: bool,
    pub leaf_non_executable: bool,
}

const PARENT_FLAGS: PageTableFlags = PageTableFlags::PRESENT
    .union(PageTableFlags::WRITABLE)
    .union(PageTableFlags::USER_ACCESSIBLE);

fn indices(address: u64) -> [usize; 4] {
    [
        ((address >> 39) & 511) as usize,
        ((address >> 30) & 511) as usize,
        ((address >> 21) & 511) as usize,
        ((address >> 12) & 511) as usize,
    ]
}

fn table_pointer(manager: &X86MemoryManager, phys: u64) -> *mut PageTable {
    manager.allocator.direct_map_pointer(phys) as *mut PageTable
}

fn validate_inactive_root(
    manager: &X86MemoryManager,
    root: PhysFrame,
) -> Result<(), MemoryError> {
    let (active, _) = Cr3::read();
    if active.start_address().as_u64() == root.start {
        return Err(MemoryError::AddressSpaceRootActive);
    }
    if !manager.allocator.is_managed_frame(root.start)
        || !manager.allocator.is_frame_allocated(root.start)
    {
        return Err(MemoryError::InvalidAddressSpaceRoot);
    }
    Ok(())
}

fn leaf_flags(permissions: PagePermissions) -> Result<PageTableFlags, MemoryError> {
    if !permissions.user() || permissions.writable() && permissions.executable()
        || permissions.cache_policy() != MemoryCachePolicy::Normal
    {
        return Err(MemoryError::InvalidProcessUserPermissions);
    }
    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if permissions.writable() {
        flags |= PageTableFlags::WRITABLE;
    }
    if !permissions.executable() {
        flags |= PageTableFlags::NO_EXECUTE;
    }
    Ok(flags)
}

pub fn map_one_inactive_user_leaf(
    root: PhysFrame,
    address: u64,
    permissions: PagePermissions,
) -> Result<InactiveUserLeaf, MemoryError> {
    crate::memory::address_space::validate_mapping_target(address, permissions)?;
    if address % PAGE_SIZE != 0 {
        return Err(MemoryError::AddressNotAligned);
    }
    let flags = leaf_flags(permissions)?;
    let idx = indices(address);

    with_manager(|manager| {
        validate_inactive_root(manager, root)?;
        // SAFETY: the process root is inactive, exclusively owned and reachable
        // only through the always-supervisor physical direct map.
        let pml4 = unsafe { &mut *table_pointer(manager, root.start) };
        if !pml4[idx[0]].is_unused() {
            return Err(MemoryError::ProcessUserLeafAlreadyMapped);
        }

        // Allocate before publishing any PML4 entry. An OOM cannot leave a
        // partially installed subtree; rollback returns each allocated frame.
        let mut allocated = [None; 4];
        for index in 0..allocated.len() {
            let Some(next) = manager.allocator.next_frame() else {
                for frame in allocated.iter().flatten().rev() {
                    manager.allocator.release_frame(*frame)?;
                }
                return Err(MemoryError::OutOfFrames);
            };
            let frame = PhysFrame {
                start: next.start_address().as_u64(),
            };
            let pointer = manager.allocator.direct_map_pointer(frame.start);
            unsafe { core::ptr::write_bytes(pointer, 0, PAGE_SIZE as usize) };
            allocated[index] = Some(frame);
        }
        let [Some(l3), Some(l2), Some(l1), Some(data_frame)] = allocated else {
            unreachable!("four process frames allocated")
        };

        unsafe {
            let table3 = &mut *table_pointer(manager, l3.start);
            let table2 = &mut *table_pointer(manager, l2.start);
            let table1 = &mut *table_pointer(manager, l1.start);
            table1[idx[3]].set_addr(PhysAddr::new(data_frame.start), flags);
            table2[idx[2]].set_addr(PhysAddr::new(l1.start), PARENT_FLAGS);
            table3[idx[1]].set_addr(PhysAddr::new(l2.start), PARENT_FLAGS);
            // Publish last, after the entire chain has been initialized.
            pml4[idx[0]].set_addr(PhysAddr::new(l3.start), PARENT_FLAGS);
        }

        Ok(InactiveUserLeaf {
            virtual_address: address,
            data_frame,
            l3,
            l2,
            l1,
            leaf_flags: flags,
        })
    })?
}

fn verify_chain(
    manager: &X86MemoryManager,
    root: PhysFrame,
    mapping: &InactiveUserLeaf,
) -> Result<(), MemoryError> {
    validate_inactive_root(manager, root)?;
    let idx = indices(mapping.virtual_address);
    // SAFETY: these tables are allocator-owned and the process PML4 is inactive.
    let pml4 = unsafe { &*table_pointer(manager, root.start) };
    let t3 = unsafe { &*table_pointer(manager, mapping.l3.start) };
    let t2 = unsafe { &*table_pointer(manager, mapping.l2.start) };
    let t1 = unsafe { &*table_pointer(manager, mapping.l1.start) };
    let links = [
        (&pml4[idx[0]], mapping.l3.start),
        (&t3[idx[1]], mapping.l2.start),
        (&t2[idx[2]], mapping.l1.start),
        (&t1[idx[3]], mapping.data_frame.start),
    ];
    for (index, (entry, expected_phys)) in links.iter().enumerate() {
        let expected_flags = if index == 3 {
            mapping.leaf_flags
        } else {
            PARENT_FLAGS
        };
        // Once this process PML4 has actually been loaded, the CPU may
        // set ACCESSED on each paging level and DIRTY on the writable leaf.
        // Preserve exact ownership/permission checks while tolerating only
        // these architecturally maintained status bits.
        let hardware_bits = if index == 3 {
            PageTableFlags::ACCESSED | PageTableFlags::DIRTY
        } else {
            PageTableFlags::ACCESSED
        };
        if entry.is_unused()
            || entry.addr().as_u64() != *expected_phys
            || (entry.flags() & !hardware_bits) != expected_flags
        {
            return Err(MemoryError::ProcessUserLeafCorrupt);
        }
    }
    if (0..PML4_LOWER_HALF_ENTRIES)
        .any(|index| index != idx[0] && !pml4[index].is_unused())
        || (0..512).any(|index| index != idx[1] && !t3[index].is_unused())
        || (0..512).any(|index| index != idx[2] && !t2[index].is_unused())
        || (0..512).any(|index| index != idx[3] && !t1[index].is_unused())
    {
        return Err(MemoryError::ProcessUserLeafCorrupt);
    }
    Ok(())
}

pub fn inspect_inactive_user_leaf(
    root: PhysFrame,
    mapping: &InactiveUserLeaf,
) -> Result<InactiveUserLeafInfo, MemoryError> {
    with_manager(|manager| {
        verify_chain(manager, root, mapping)?;
        Ok(InactiveUserLeafInfo {
            root_frame: root,
            virtual_address: mapping.virtual_address,
            data_frame: mapping.data_frame,
            ancestor_user_accessible: PARENT_FLAGS.contains(PageTableFlags::USER_ACCESSIBLE),
            leaf_user_accessible: mapping.leaf_flags.contains(PageTableFlags::USER_ACCESSIBLE),
            leaf_writable: mapping.leaf_flags.contains(PageTableFlags::WRITABLE),
            leaf_non_executable: mapping.leaf_flags.contains(PageTableFlags::NO_EXECUTE),
        })
    })?
}

pub fn destroy_inactive_user_leaf(
    root: PhysFrame,
    mapping: InactiveUserLeaf,
) -> Result<(), MemoryError> {
    with_manager(|manager| {
        verify_chain(manager, root, &mapping)?;

        let idx = indices(mapping.virtual_address);
        // SAFETY: inactive process mapping with no user executor or DMA owner.
        // Detach from its root before returning any descendant frame.
        let pml4 = unsafe { &mut *table_pointer(manager, root.start) };
        pml4[idx[0]].set_unused();

        for frame in [mapping.data_frame, mapping.l1, mapping.l2, mapping.l3] {
            manager.allocator.release_frame(frame)?;
        }
        Ok(())
    })?
}

#[cfg(feature = "m5e-ci-self-test")]
pub fn ci_probe_inactive_user_leaf(
    root: PhysFrame,
    mapping: &InactiveUserLeaf,
    pattern: u64,
) -> Result<bool, MemoryError> {
    with_manager(|manager| {
        verify_chain(manager, root, mapping)?;
        let pointer = manager.allocator.direct_map_pointer(mapping.data_frame.start);
        // No process CR3 switch occurs: staging and checking use the kernel
        // physical direct map while the process leaf remains inactive.
        unsafe {
            core::ptr::write_volatile(pointer as *mut u64, pattern);
            Ok(core::ptr::read_volatile(pointer as *const u64) == pattern)
        }
    })?
}


#[cfg(feature = "m5f-ci-self-test")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlledCr3Proof {
    pub kernel_cr3_before: u64,
    pub process_cr3_observed: u64,
    pub kernel_cr3_after: u64,
    pub virtual_readback: u64,
    pub physical_readback: u64,
    pub absent_from_kernel_root: bool,
}

#[cfg(feature = "m5f-ci-self-test")]
pub fn ci_controlled_cr3_roundtrip(
    root: PhysFrame,
    mapping: &InactiveUserLeaf,
    pattern: u64,
) -> Result<ControlledCr3Proof, MemoryError> {
    if crate::arch::in_interrupt() {
        return Err(MemoryError::InvalidAddressSpaceRoot);
    }

    // Validate all four links, exact ownership and U+RW/NX before switching.
    // The memory manager lock is released before loading another CR3.
    let info = inspect_inactive_user_leaf(root, mapping)?;
    if info.root_frame != root
        || !info.ancestor_user_accessible
        || !info.leaf_user_accessible
        || !info.leaf_writable
        || !info.leaf_non_executable
    {
        return Err(MemoryError::InvalidProcessUserPermissions);
    }

    let absent_from_kernel_root = with_manager(|manager| {
        matches!(
            manager.mapper.translate(VirtAddr::new(mapping.virtual_address)),
            TranslateResult::NotMapped
        )
    })?;
    if !absent_from_kernel_root {
        return Err(MemoryError::PageAlreadyMapped);
    }

    // The only CR3 switch takes place with IF=0 on the bootstrap CPU. No
    // scheduler handoff, Rust callback, allocator or manager lock is active
    // between the two CR3 writes.
    let result = interrupts::without_interrupts(|| {
        let (before, _) = Cr3::read();
        let kernel_cr3_before = before.start_address().as_u64();
        if kernel_cr3_before == root.start {
            return Err(MemoryError::AddressSpaceRootActive);
        }

        // SAFETY: the inactive mapping and shared kernel-half entries were
        // validated above, its process object remains alive throughout this
        // function, and interrupts are disabled until kernel CR3 is restored.
        let observed = unsafe {
            super::process_cr3_probe::execute(
                root.start,
                mapping.virtual_address,
                pattern,
            )
        };

        let (actual_after, _) = Cr3::read();
        let kernel_cr3_after = actual_after.start_address().as_u64();
        Ok((
            kernel_cr3_before,
            observed,
            kernel_cr3_after,
        ))
    })?;

    let (kernel_cr3_before, observed, kernel_cr3_after) = result;
    // All checks happen *after* the assembly has already restored CR3.
    if observed.process_cr3_observed != root.start
        || observed.kernel_cr3_restored & !0xfff != kernel_cr3_before
        || kernel_cr3_after != kernel_cr3_before
        || observed.virtual_readback != pattern
    {
        return Err(MemoryError::ProcessUserLeafCorrupt);
    }

    let physical_readback = with_manager(|manager| {
        let ptr = manager.allocator.direct_map_pointer(mapping.data_frame.start);
        unsafe { core::ptr::read_volatile(ptr as *const u64) }
    })?;

    if physical_readback != pattern {
        return Err(MemoryError::SelfTestDataMismatch);
    }

    Ok(ControlledCr3Proof {
        kernel_cr3_before,
        process_cr3_observed: observed.process_cr3_observed,
        kernel_cr3_after,
        virtual_readback: observed.virtual_readback,
        physical_readback,
        absent_from_kernel_root,
    })
}


#[cfg(feature = "m5g-ci-self-test")]
pub fn ci_process_ring3_roundtrip(
    root: PhysFrame,
    mapping: &InactiveUserLeaf,
) -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError> {
    let info = inspect_inactive_user_leaf(root, mapping)?;
    if !info.ancestor_user_accessible
        || !info.leaf_user_accessible
        || info.leaf_writable
        || !mapping.leaf_flags.contains(PageTableFlags::PRESENT)
        || mapping.leaf_flags.contains(PageTableFlags::NO_EXECUTE)
        || mapping.virtual_address % PAGE_SIZE != 0
    {
        return Err(MemoryError::InvalidProcessUserPermissions);
    }

    // Demonstrate that this virtual address is provided by the process root,
    // rather than accidentally inherited from the active kernel root.
    let absent_from_kernel = with_manager(|manager| {
        matches!(
            manager.mapper.translate(VirtAddr::new(mapping.virtual_address)),
            TranslateResult::NotMapped
        )
    })?;
    if !absent_from_kernel {
        return Err(MemoryError::PageAlreadyMapped);
    }

    // Stage the one-shot instruction bytes through the kernel direct map
    // while the process is INACTIVE. The user leaf remains U+RX, never U+RWX.
    // The stub never pushes or stores to the user stack: M5-G deliberately
    // preserves M5-E's one-leaf limit.
    with_manager(|manager| {
        verify_chain(manager, root, mapping)?;
        let ptr = manager.allocator.direct_map_pointer(mapping.data_frame.start);
        let bytes = crate::arch::process_ring3::stub_bytes();
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        }
        Ok(())
    })??;

    let user_rsp = mapping.virtual_address + PAGE_SIZE - 16;
    // SAFETY: the ProcessObject holds this root and the executable leaf
    // alive, this single-leaf code stub is installed, and no task is running
    // with this process CR3. The assembly restores kernel CR3 before Rust.
    let proof = unsafe {
        crate::arch::process_ring3::enter_once(
            root.start, mapping.virtual_address, user_rsp
        )
    };

    // The CPU may have set ACCESSED status bits on the now-activated tables.
    // The M5-F verification still checks ownership and permissions exactly.
    let after = inspect_inactive_user_leaf(root, mapping)?;
    if proof.observed_process_cr3 & !0xfff != root.start
        || proof.original_kernel_cr3 != proof.restored_kernel_cr3
        || proof.frame_address == 0
        || !after.leaf_user_accessible
        || after.leaf_writable
        || !after.ancestor_user_accessible
        || after.leaf_non_executable
    {
        return Err(MemoryError::ProcessUserLeafCorrupt);
    }
    Ok(proof)
}


#[cfg(feature = "m5g-ci-self-test")]
impl InactiveUserLeaf {
    pub fn ci_ring3_on_process_root(
        &self,
        root: PhysFrame,
    ) -> Result<crate::arch::process_ring3::Ring3ProcessProof, MemoryError> {
        ci_process_ring3_roundtrip(root, self)
    }
}

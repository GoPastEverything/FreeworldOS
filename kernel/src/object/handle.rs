use alloc::sync::Arc;
use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    ops::{BitOr, BitOrAssign},
    sync::atomic::{AtomicBool, Ordering},
};

use super::FwObject;

pub const MAX_HANDLES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct Handle(pub u64);

impl Handle {
    pub const INVALID: Self = Self(0);

    fn from_parts(slot: usize, generation: u32) -> Self {
        Self((u64::from(generation) << 32) | (slot as u64 + 1))
    }

    fn parts(self) -> Option<(usize, u32)> {
        let encoded_slot = (self.0 & 0xffff_ffff) as u32;
        if encoded_slot == 0 {
            return None;
        }

        let slot = encoded_slot as usize - 1;
        let generation = (self.0 >> 32) as u32;
        if slot >= MAX_HANDLES || generation == 0 {
            return None;
        }

        Some((slot, generation))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct Rights(u32);

impl Rights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const ALL: Self = Self(Self::READ.0 | Self::WRITE.0);

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl BitOr for Rights {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Rights {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandleError {
    NotInitialized,
    AlreadyInitialized,
    TableFull,
    InvalidHandle,
    AccessDenied {
        required: Rights,
        granted: Rights,
    },
}

struct HandleEntry {
    object: Arc<FwObject>,
    rights: Rights,
}

struct HandleSlot {
    generation: u32,
    entry: Option<HandleEntry>,
}

struct HandleTable {
    slots: [HandleSlot; MAX_HANDLES],
}

impl HandleTable {
    fn new() -> Self {
        Self {
            slots: core::array::from_fn(|_| HandleSlot {
                generation: 1,
                entry: None,
            }),
        }
    }

    fn insert(
        &mut self,
        object: Arc<FwObject>,
        rights: Rights,
    ) -> Result<Handle, HandleError> {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.entry.is_none() {
                let handle = Handle::from_parts(index, slot.generation);
                slot.entry = Some(HandleEntry { object, rights });
                return Ok(handle);
            }
        }

        Err(HandleError::TableFull)
    }

    fn get(
        &self,
        handle: Handle,
        required: Rights,
    ) -> Result<Arc<FwObject>, HandleError> {
        let (index, generation) = handle.parts().ok_or(HandleError::InvalidHandle)?;
        let slot = &self.slots[index];

        if slot.generation != generation {
            return Err(HandleError::InvalidHandle);
        }

        let entry = slot.entry.as_ref().ok_or(HandleError::InvalidHandle)?;
        if !entry.rights.contains(required) {
            return Err(HandleError::AccessDenied {
                required,
                granted: entry.rights,
            });
        }

        Ok(Arc::clone(&entry.object))
    }

    fn close(&mut self, handle: Handle) -> Result<Arc<FwObject>, HandleError> {
        let (index, generation) = handle.parts().ok_or(HandleError::InvalidHandle)?;
        let slot = &mut self.slots[index];

        if slot.generation != generation {
            return Err(HandleError::InvalidHandle);
        }

        let entry = slot.entry.take().ok_or(HandleError::InvalidHandle)?;
        slot.generation = next_generation(slot.generation);
        Ok(entry.object)
    }
}

const fn next_generation(current: u32) -> u32 {
    let next = current.wrapping_add(1);
    if next == 0 { 1 } else { next }
}

struct TableStorage(UnsafeCell<MaybeUninit<HandleTable>>);

// SAFETY: All access is serialized by TABLE_LOCK and M3 is still single-core.
// Interrupt handlers do not touch the object/handle subsystem.
unsafe impl Sync for TableStorage {}

static TABLE: TableStorage = TableStorage(UnsafeCell::new(MaybeUninit::uninit()));
static TABLE_INITIALIZED: AtomicBool = AtomicBool::new(false);
static TABLE_LOCK: AtomicBool = AtomicBool::new(false);

struct TableGuard;

impl TableGuard {
    fn acquire() -> Self {
        while TABLE_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        Self
    }
}

impl Drop for TableGuard {
    fn drop(&mut self) {
        TABLE_LOCK.store(false, Ordering::Release);
    }
}

pub fn init() -> Result<(), HandleError> {
    let _guard = TableGuard::acquire();

    if TABLE_INITIALIZED.load(Ordering::Acquire) {
        return Err(HandleError::AlreadyInitialized);
    }

    // SAFETY: The table is written exactly once while the table lock is held.
    unsafe {
        (*TABLE.0.get()).write(HandleTable::new());
    }
    TABLE_INITIALIZED.store(true, Ordering::Release);
    Ok(())
}

pub fn insert(
    object: Arc<FwObject>,
    rights: Rights,
) -> Result<Handle, HandleError> {
    with_table(|table| table.insert(object, rights))?
}

pub fn get(
    handle: Handle,
    required: Rights,
) -> Result<Arc<FwObject>, HandleError> {
    with_table(|table| table.get(handle, required))?
}

pub fn close(handle: Handle) -> Result<(), HandleError> {
    let object = with_table(|table| table.close(handle))??;

    // Drop the final table reference after releasing TABLE_LOCK so object
    // destruction and heap deallocation never happen while the table is locked.
    drop(object);
    Ok(())
}

fn with_table<R>(
    operation: impl FnOnce(&mut HandleTable) -> R,
) -> Result<R, HandleError> {
    let _guard = TableGuard::acquire();

    if !TABLE_INITIALIZED.load(Ordering::Acquire) {
        return Err(HandleError::NotInitialized);
    }

    // SAFETY: Initialization completed before publication and TABLE_LOCK
    // serializes all mutable access.
    let table = unsafe { &mut *(*TABLE.0.get()).as_mut_ptr() };
    Ok(operation(table))
}

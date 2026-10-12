//! M5-M test-only deterministic two-slot selection.
//! No timer, task scheduler, or callgate. Slots are explicit kernel test IDs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Slot { A, B }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase { Suspended, Runnable, Finished }

pub struct TwoSlotQueue {
    slots: [Phase; 2],
    picks: u8,
}

impl TwoSlotQueue {
    pub fn new_suspended() -> Self {
        Self { slots: [Phase::Suspended, Phase::Suspended], picks: 0 }
    }

    /// Explicitly pick A, then B, then A. Each call is deliberate, never a timer.
    /// The caller must prove that the chosen slot's saved private kernel frame
    /// is still owned by that process and RSP0 is installed before IRETQ.
    pub fn pick_next(&mut self) -> Result<Slot, &'static str> {
        let (slot, expected) = match self.picks {
            0 => (Slot::A, Phase::Suspended),
            1 => (Slot::B, Phase::Suspended),
            2 => (Slot::A, Phase::Runnable),
            _ => return Err("two-slot queue exhausted"),
        };
        let index = match slot { Slot::A => 0, Slot::B => 1 };
        if self.slots[index] != expected { return Err("slot not resumable"); }
        self.picks += 1;
        self.slots[index] = if self.picks == 3 { Phase::Finished } else { Phase::Runnable };
        Ok(slot)
    }

    pub fn picks(&self) -> u8 { self.picks }
    pub fn phase(&self, slot: Slot) -> Phase {
        self.slots[match slot { Slot::A => 0, Slot::B => 1 }]
    }
}

#[cfg(feature = "m5m-ci-self-test")]
pub fn ci_self_test() -> Result<(), &'static str> {
    let mut q = TwoSlotQueue::new_suspended();
    if q.pick_next()? != Slot::A || q.pick_next()? != Slot::B
        || q.pick_next()? != Slot::A || q.pick_next().is_ok()
        || q.phase(Slot::A) != Phase::Finished || q.phase(Slot::B) != Phase::Runnable
        || q.picks() != 3
    { return Err("explicit two-slot order violated"); }
    crate::arch::serial::println(
        "FreeWorldOS: M5-M explicit queue unit proof: passed picks=A,B,A timer_choice=off scheduler=off callgate=off"
    );
    Ok(())
}

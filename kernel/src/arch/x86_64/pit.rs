const PIT_CHANNEL2_DATA: u16 = 0x42;
const PIT_COMMAND: u16 = 0x43;
const SPEAKER_CONTROL: u16 = 0x61;

pub const PIT_HZ: u64 = 1_193_182;
pub const CALIBRATION_COUNT: u16 = 11_932;

const CHANNEL2_LOHI_MODE0_BINARY: u8 = 0xB0;
const GATE2: u8 = 1 << 0;
const SPEAKER_ENABLE: u8 = 1 << 1;
const OUT2: u8 = 1 << 5;
const MAX_POLL_SPINS: usize = 5_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PitError {
    ZeroCount,
    OutputDidNotArm,
    Timeout,
}

#[derive(Clone, Copy, Debug)]
pub struct PreparedWindow {
    base_control: u8,
}

pub fn prepare_channel2_window(count: u16) -> Result<PreparedWindow, PitError> {
    if count == 0 {
        return Err(PitError::ZeroCount);
    }

    let base = unsafe { inb(SPEAKER_CONTROL) } & !(GATE2 | SPEAKER_ENABLE);

    // Hold gate 2 low and keep the PC speaker disconnected while programming.
    unsafe {
        outb(SPEAKER_CONTROL, base);
        outb(PIT_COMMAND, CHANNEL2_LOHI_MODE0_BINARY);
        outb(PIT_CHANNEL2_DATA, count as u8);
        outb(PIT_CHANNEL2_DATA, (count >> 8) as u8);
    }

    // Mode 0 drives OUT2 low once the count is loaded. Seeing it low before
    // starting the gate avoids accepting a stale terminal-count state.
    for _ in 0..MAX_POLL_SPINS {
        if unsafe { inb(SPEAKER_CONTROL) } & OUT2 == 0 {
            return Ok(PreparedWindow {
                base_control: base,
            });
        }
        core::hint::spin_loop();
    }

    Err(PitError::OutputDidNotArm)
}

pub fn run_prepared_window(window: PreparedWindow) -> Result<(), PitError> {
    // Raising GATE2 starts the one-shot countdown. No PIT interrupt is enabled;
    // completion is observed only through the OUT2 status bit.
    unsafe {
        outb(SPEAKER_CONTROL, window.base_control | GATE2);
    }

    for _ in 0..MAX_POLL_SPINS {
        if unsafe { inb(SPEAKER_CONTROL) } & OUT2 != 0 {
            unsafe { outb(SPEAKER_CONTROL, window.base_control) };
            return Ok(());
        }
        core::hint::spin_loop();
    }

    unsafe { outb(SPEAKER_CONTROL, window.base_control) };
    Err(PitError::Timeout)
}

unsafe fn outb(port: u16, value: u8) {
    // SAFETY: Caller supplies a valid PC-compatible PIT/speaker I/O port.
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") value,
            options(nomem, nostack, preserves_flags)
        );
    }
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    // SAFETY: Caller supplies a valid PC-compatible PIT/speaker I/O port.
    unsafe {
        core::arch::asm!(
            "in al, dx",
            out("al") value,
            in("dx") port,
            options(nomem, nostack, preserves_flags)
        );
    }
    value
}

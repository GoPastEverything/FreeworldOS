use core::{
    cell::UnsafeCell,
    fmt::{self, Write},
    panic::PanicInfo,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::{
    arch,
    debug::{backtrace, events},
};

const PANIC_MESSAGE_CAPACITY: usize = 256;
const RECENT_EVENT_DUMP: usize = 24;

static PANIC_ACTIVE: AtomicBool = AtomicBool::new(false);
static PANIC_MESSAGE_LEN: AtomicUsize = AtomicUsize::new(0);
static PANIC_MESSAGE_TRUNCATED: AtomicBool = AtomicBool::new(false);

struct PanicMessageStorage(UnsafeCell<[u8; PANIC_MESSAGE_CAPACITY]>);

// SAFETY: Only the first panic/fatal path writes the buffer after winning
// PANIC_ACTIVE. Reentrant panic entries do not touch it.
unsafe impl Sync for PanicMessageStorage {}

static PANIC_MESSAGE: PanicMessageStorage =
    PanicMessageStorage(UnsafeCell::new([0; PANIC_MESSAGE_CAPACITY]));

struct FixedText {
    bytes: [u8; PANIC_MESSAGE_CAPACITY],
    len: usize,
    truncated: bool,
}

impl FixedText {
    const fn new() -> Self {
        Self {
            bytes: [0; PANIC_MESSAGE_CAPACITY],
            len: 0,
            truncated: false,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl Write for FixedText {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let available = PANIC_MESSAGE_CAPACITY.saturating_sub(self.len);
        let copy_len = available.min(text.len());

        if copy_len != 0 {
            self.bytes[self.len..self.len + copy_len]
                .copy_from_slice(&text.as_bytes()[..copy_len]);
            self.len += copy_len;
        }

        if copy_len != text.len() {
            self.truncated = true;
        }

        Ok(())
    }
}

pub fn is_active() -> bool {
    PANIC_ACTIVE.load(Ordering::Acquire)
}

pub fn panic(info: &PanicInfo<'_>) -> ! {
    arch::disable_interrupts();

    if PANIC_ACTIVE.swap(true, Ordering::AcqRel) {
        panic_reentry_halt();
    }

    let mut message = FixedText::new();
    let _ = write!(&mut message, "{}", info.message());
    publish_message(&message);

    events::record(
        events::KERNEL_PANIC,
        events::SUBSYSTEM_EXCEPTION,
        events::LEVEL_FATAL,
        [
            message.len as u64,
            message.truncated as u64,
            panic_location_line(info),
            panic_location_column(info),
        ],
    );

    arch::serial::write_raw(b"FreeWorldOS: KERNEL PANIC\r\n");
    arch::serial::write_raw(b"FreeWorldOS: PANIC message=");
    arch::serial::write_raw(message.as_bytes());
    if message.truncated {
        arch::serial::write_raw(b" [truncated]");
    }
    arch::serial::write_raw(b"\r\n");

    if let Some(location) = info.location() {
        arch::serial::write_fmt(format_args!(
            "FreeWorldOS: PANIC location={}:{}:{}\n",
            location.file(),
            location.line(),
            location.column(),
        ));
    }

    dump_current_registers();
    backtrace::dump_to_serial(None);
    events::dump_recent_to_serial(RECENT_EVENT_DUMP);
    arch::halt_loop()
}

pub fn fatal_exception(
    event_id: events::EventId,
    label: &'static str,
    instruction_pointer: u64,
    stack_pointer: u64,
    error_code: u64,
    fault_address: u64,
) -> ! {
    arch::disable_interrupts();

    if PANIC_ACTIVE.swap(true, Ordering::AcqRel) {
        panic_reentry_halt();
    }

    events::record(
        event_id,
        events::SUBSYSTEM_EXCEPTION,
        events::LEVEL_FATAL,
        [
            instruction_pointer,
            stack_pointer,
            error_code,
            fault_address,
        ],
    );

    arch::serial::write_raw(b"FreeWorldOS: FATAL\r\n");
    arch::serial::write_fmt(format_args!(
        "FreeWorldOS: EXCEPTION: {label} ip={instruction_pointer:#x} sp={stack_pointer:#x} error={error_code:#x} fault={fault_address:#x}\n"
    ));

    dump_current_registers();
    backtrace::dump_to_serial(Some(instruction_pointer));
    events::dump_recent_to_serial(RECENT_EVENT_DUMP);
    arch::halt_loop()
}

fn publish_message(message: &FixedText) {
    // SAFETY: Only the first panic path reaches this function.
    unsafe {
        let storage = &mut *PANIC_MESSAGE.0.get();
        storage.fill(0);
        storage[..message.len].copy_from_slice(message.as_bytes());
    }

    PANIC_MESSAGE_LEN.store(message.len, Ordering::Release);
    PANIC_MESSAGE_TRUNCATED.store(message.truncated, Ordering::Release);
}

fn panic_reentry_halt() -> ! {
    // Deliberately bypass formatting, event recording, heap, memory-manager
    // operations, and every lock. If panic diagnostics themselves fail, this
    // is the final non-recursive path.
    arch::serial::write_raw(
        b"FreeWorldOS: PANIC REENTRY - diagnostic path faulted\r\n",
    );
    arch::halt_loop()
}

fn dump_current_registers() {
    let rsp: u64;
    let rbp: u64;
    let rflags: u64;

    // SAFETY: These instructions only snapshot architectural registers.
    unsafe {
        core::arch::asm!(
            "mov {rsp_out}, rsp",
            "mov {rbp_out}, rbp",
            "pushfq",
            "pop {rflags_out}",
            rsp_out = out(reg) rsp,
            rbp_out = out(reg) rbp,
            rflags_out = out(reg) rflags,
            options(preserves_flags),
        );
    }

    arch::serial::write_fmt(format_args!(
        "FreeWorldOS: REGISTERS rsp={rsp:#018x} rbp={rbp:#018x} rflags={rflags:#018x}\n"
    ));
}

fn panic_location_line(info: &PanicInfo<'_>) -> u64 {
    info.location()
        .map(|location| u64::from(location.line()))
        .unwrap_or(0)
}

fn panic_location_column(info: &PanicInfo<'_>) -> u64 {
    info.location()
        .map(|location| u64::from(location.column()))
        .unwrap_or(0)
}

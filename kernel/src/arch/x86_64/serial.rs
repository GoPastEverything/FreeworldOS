use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};

const COM1: u16 = 0x3F8;
static READY: AtomicBool = AtomicBool::new(false);

pub fn init() {
    // SAFETY: These ports are the legacy COM1 UART on x86_64 PC-compatible systems.
    unsafe {
        outb(COM1 + 1, 0x00); // Disable interrupts.
        outb(COM1 + 3, 0x80); // Enable DLAB.
        outb(COM1, 0x03);     // Divisor 3 = 38400 baud.
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03); // 8 data bits, no parity, one stop bit.
        outb(COM1 + 2, 0xC7); // FIFO on, clear, 14-byte threshold.
        outb(COM1 + 4, 0x0B); // IRQs enabled, RTS/DSR set.
    }
    READY.store(true, Ordering::Release);
}

pub fn println(message: &str) {
    write_fmt(format_args!("{message}\n"));
}

pub fn write_fmt(args: fmt::Arguments<'_>) {
    if !READY.load(Ordering::Acquire) {
        return;
    }
    let mut writer = SerialWriter;
    let _ = writer.write_fmt(args);
}

struct SerialWriter;

impl Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                write_byte(b'\r');
            }
            write_byte(byte);
        }
        Ok(())
    }
}

fn write_byte(byte: u8) {
    while unsafe { inb(COM1 + 5) } & 0x20 == 0 {
        core::hint::spin_loop();
    }
    unsafe { outb(COM1, byte) };
}

unsafe fn outb(port: u16, value: u8) {
    // SAFETY: Caller guarantees that the port is valid for byte output.
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") value,
            options(nomem, nostack, preserves_flags)
        )
    };
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    // SAFETY: Caller guarantees that the port is valid for byte input.
    unsafe {
        core::arch::asm!(
            "in al, dx",
            out("al") value,
            in("dx") port,
            options(nomem, nostack, preserves_flags)
        )
    };
    value
}

const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_COMMAND: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

const ICW1_INIT: u8 = 0x10;
const ICW1_ICW4: u8 = 0x01;
const ICW4_8086: u8 = 0x01;

pub const MASTER_VECTOR_OFFSET: u8 = 0x20;
pub const SLAVE_VECTOR_OFFSET: u8 = 0x28;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PicError {
    MaskVerificationFailed { master: u8, slave: u8 },
}

/// Remaps the legacy 8259 PICs away from CPU exception vectors and masks every
/// IRQ line. Interrupts must remain disabled while this runs.
pub fn remap_and_mask() -> Result<(), PicError> {
    // Mask first, before beginning the ICW sequence.
    unsafe {
        outb(PIC1_DATA, 0xFF);
        outb(PIC2_DATA, 0xFF);

        outb(PIC1_COMMAND, ICW1_INIT | ICW1_ICW4);
        io_wait();
        outb(PIC2_COMMAND, ICW1_INIT | ICW1_ICW4);
        io_wait();

        outb(PIC1_DATA, MASTER_VECTOR_OFFSET);
        io_wait();
        outb(PIC2_DATA, SLAVE_VECTOR_OFFSET);
        io_wait();

        // Master has a slave on IRQ2; slave identity is cascade input 2.
        outb(PIC1_DATA, 1 << 2);
        io_wait();
        outb(PIC2_DATA, 2);
        io_wait();

        outb(PIC1_DATA, ICW4_8086);
        io_wait();
        outb(PIC2_DATA, ICW4_8086);
        io_wait();

        // Leave both PICs fully masked after remapping.
        outb(PIC1_DATA, 0xFF);
        outb(PIC2_DATA, 0xFF);
    }

    let master = unsafe { inb(PIC1_DATA) };
    let slave = unsafe { inb(PIC2_DATA) };

    if master != 0xFF || slave != 0xFF {
        return Err(PicError::MaskVerificationFailed { master, slave });
    }

    Ok(())
}

unsafe fn outb(port: u16, value: u8) {
    // SAFETY: The caller supplies a valid legacy x86 I/O port.
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
    // SAFETY: The caller supplies a valid legacy x86 I/O port.
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

unsafe fn io_wait() {
    // Port 0x80 is conventionally used for a tiny delay on PC-compatible
    // hardware and is harmless for this early 8259 programming sequence.
    unsafe { outb(0x80, 0) };
}

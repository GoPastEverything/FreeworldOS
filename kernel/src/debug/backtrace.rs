use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch;

pub const MAX_FRAMES: usize = 32;
const MAX_STACK_SPAN: u64 = 1024 * 1024;

static KERNEL_START: AtomicU64 = AtomicU64::new(0);
static KERNEL_END: AtomicU64 = AtomicU64::new(0);

pub fn init(kernel_start: u64, kernel_len: u64) {
    let kernel_end = kernel_start
        .checked_add(kernel_len)
        .expect("FreeWorld kernel virtual range overflow");

    KERNEL_START.store(kernel_start, Ordering::Release);
    KERNEL_END.store(kernel_end, Ordering::Release);
}

#[derive(Clone, Copy)]
pub struct Backtrace {
    frames: [u64; MAX_FRAMES],
    len: usize,
}

impl Backtrace {
    pub const fn empty() -> Self {
        Self {
            frames: [0; MAX_FRAMES],
            len: 0,
        }
    }

    fn push(&mut self, address: u64) {
        if self.len < MAX_FRAMES {
            self.frames[self.len] = address;
            self.len += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn iter(&self) -> impl Iterator<Item = &u64> {
        self.frames[..self.len].iter()
    }
}

pub fn capture(prefix_ip: Option<u64>) -> Backtrace {
    let mut trace = Backtrace::empty();

    if let Some(ip) = prefix_ip {
        if is_kernel_address(ip) {
            trace.push(ip);
        }
    }

    let (mut frame_pointer, stack_pointer) = current_frame_and_stack_pointer();
    let stack_limit = stack_pointer.saturating_add(MAX_STACK_SPAN);

    while trace.len < MAX_FRAMES {
        if frame_pointer == 0
            || frame_pointer & 0x7 != 0
            || !is_canonical(frame_pointer)
            || frame_pointer < stack_pointer
            || frame_pointer.saturating_add(16) > stack_limit
        {
            break;
        }

        let frame = frame_pointer as *const u64;

        // SAFETY: Frame pointers are forced on for the kernel. We only follow
        // aligned canonical pointers that remain within a conservative 1 MiB
        // window above the current stack pointer. This is a best-effort panic
        // diagnostic, not a memory-safety authority.
        let previous = unsafe { core::ptr::read_volatile(frame) };
        let return_address = unsafe { core::ptr::read_volatile(frame.add(1)) };

        if is_kernel_address(return_address) {
            trace.push(return_address);
        }

        if previous <= frame_pointer
            || previous & 0x7 != 0
            || !is_canonical(previous)
            || previous > stack_limit
        {
            break;
        }

        frame_pointer = previous;
    }

    trace
}

pub fn dump_to_serial(prefix_ip: Option<u64>) {
    let trace = capture(prefix_ip);
    arch::serial::write_fmt(format_args!(
        "FreeWorldOS: BACKTRACE begin frames={}\n",
        trace.len()
    ));

    for (index, address) in trace.iter().enumerate() {
        arch::serial::write_fmt(format_args!(
            "FreeWorldOS: BT[{index:02}]=0x{address:016x}\n"
        ));
    }

    arch::serial::println("FreeWorldOS: BACKTRACE end");
}

fn current_frame_and_stack_pointer() -> (u64, u64) {
    let frame_pointer: u64;
    let stack_pointer: u64;

    // SAFETY: Reading RBP/RSP has no side effects.
    unsafe {
        core::arch::asm!(
            "mov {frame}, rbp",
            "mov {stack}, rsp",
            frame = out(reg) frame_pointer,
            stack = out(reg) stack_pointer,
            options(nomem, nostack, preserves_flags),
        );
    }

    (frame_pointer, stack_pointer)
}

const fn is_canonical(address: u64) -> bool {
    let upper = address >> 48;
    upper == 0 || upper == 0xffff
}


fn is_kernel_address(address: u64) -> bool {
    let start = KERNEL_START.load(Ordering::Acquire);
    let end = KERNEL_END.load(Ordering::Acquire);

    start != 0
        && end > start
        && address >= start
        && address < end
        && is_canonical(address)
}

use core::{
    arch::global_asm,
    mem::size_of,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use x86_64::{
    registers::segmentation::{Segment, CS, SS},
    VirtAddr,
};

use crate::memory::{self, PagePermissions};

use super::{interrupt_context, serial};

pub const RETURN_VECTOR: u8 = 0xF1;

const USER_CODE_PAGE: u64 = 0x0000_4000_1000_0000;
const USER_STACK_PAGE: u64 = 0x0000_4000_1000_1000;
const USER_MARKER: u64 = 0x4d35_4355_5345_5252;

const USER_CODE_BYTES: [u8; 15] = [
    0x48, 0xb8,
    (USER_MARKER >> 0) as u8,
    (USER_MARKER >> 8) as u8,
    (USER_MARKER >> 16) as u8,
    (USER_MARKER >> 24) as u8,
    (USER_MARKER >> 32) as u8,
    (USER_MARKER >> 40) as u8,
    (USER_MARKER >> 48) as u8,
    (USER_MARKER >> 56) as u8,
    0x50,
    0xcd, RETURN_VECTOR,
    0x0f, 0x0b,
];

const USER_INT_RETURN_OFFSET: u64 = 13;

static ARMED: AtomicBool = AtomicBool::new(false);
static RETURN_SEEN: AtomicBool = AtomicBool::new(false);
static EXPECTED_USER_RIP: AtomicU64 = AtomicU64::new(0);
static EXPECTED_USER_RSP: AtomicU64 = AtomicU64::new(0);
static OBSERVED_FRAME_ADDRESS: AtomicU64 = AtomicU64::new(0);

#[repr(C)]
struct UserReturnFrame {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rbp: u64,
    rdi: u64,
    rsi: u64,
    rdx: u64,
    rcx: u64,
    rbx: u64,
    rax: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

const USER_RETURN_FRAME_BYTES: usize = size_of::<UserReturnFrame>();

const _: () = {
    assert!(USER_RETURN_FRAME_BYTES == 160);
};

global_asm!(
    r#"
    .intel_syntax noprefix

    .section .bss
    .align 8
__freeworld_m5c_saved_kernel_rsp:
    .quad 0

    .section .text

    .global __freeworld_m5c_enter_user
    .type __freeworld_m5c_enter_user,@function
__freeworld_m5c_enter_user:
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15

    mov qword ptr [rip + __freeworld_m5c_saved_kernel_rsp], rsp

    push rcx
    push rsi
    push 2
    push rdx
    push rdi
    iretq

__freeworld_m5c_resume_kernel:
    mov eax, 1
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx
    ret

    .global __freeworld_m5c_return_entry
    .type __freeworld_m5c_return_entry,@function
__freeworld_m5c_return_entry:
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    mov rdi, rsp
    cld
    call __freeworld_m5c_return_dispatch

    mov rsp, qword ptr [rip + __freeworld_m5c_saved_kernel_rsp]
    jmp __freeworld_m5c_resume_kernel
    "#
);

unsafe extern "C" {
    fn __freeworld_m5c_enter_user(
        user_rip: u64,
        user_rsp: u64,
        user_cs: u64,
        user_ss: u64,
    ) -> u64;
    fn __freeworld_m5c_return_entry();
}

pub fn handler_addr() -> VirtAddr {
    VirtAddr::new(__freeworld_m5c_return_entry as usize as u64)
}

pub fn ci_self_test() {
    assert!(
        !super::interrupts_enabled(),
        "M5-C requires IF clear before controlled ring-3 entry"
    );
    assert!(
        !interrupt_context::in_interrupt(),
        "M5-C cannot begin inside an interrupt handler"
    );

    let code_frame = memory::allocate_frame()
        .expect("M5-C failed to allocate user code frame");
    let stack_frame = memory::allocate_frame()
        .expect("M5-C failed to allocate user stack frame");

    unsafe {
        memory::map_page(
            USER_CODE_PAGE,
            code_frame,
            PagePermissions::user_read_write(),
        )
        .expect("M5-C failed to stage user code page");

        core::ptr::copy_nonoverlapping(
            USER_CODE_BYTES.as_ptr(),
            USER_CODE_PAGE as *mut u8,
            USER_CODE_BYTES.len(),
        );
    }

    let staged_code = memory::unmap_page(USER_CODE_PAGE)
        .expect("M5-C failed to unmap staged user code");
    assert_eq!(staged_code, code_frame);

    unsafe {
        memory::map_page(
            USER_CODE_PAGE,
            code_frame,
            PagePermissions::user_read_execute(),
        )
        .expect("M5-C failed to map user code RX");

        memory::map_page(
            USER_STACK_PAGE,
            stack_frame,
            PagePermissions::user_read_write(),
        )
        .expect("M5-C failed to map user stack RW");
    }

    let user_stack_top = USER_STACK_PAGE + memory::PAGE_SIZE;
    let initial_user_rsp = user_stack_top - 16;
    let expected_interrupt_rsp = initial_user_rsp - 8;
    let expected_interrupt_rip = USER_CODE_PAGE + USER_INT_RETURN_OFFSET;

    EXPECTED_USER_RSP.store(expected_interrupt_rsp, Ordering::Release);
    EXPECTED_USER_RIP.store(expected_interrupt_rip, Ordering::Release);
    RETURN_SEEN.store(false, Ordering::Release);
    OBSERVED_FRAME_ADDRESS.store(0, Ordering::Release);
    assert!(
        !ARMED.swap(true, Ordering::AcqRel),
        "M5-C round-trip was already armed"
    );

    let user_cs = u64::from(super::user_code_selector().0);
    let user_ss = u64::from(super::user_data_selector().0);

    serial::write_fmt(format_args!(
        "FreeWorldOS: M5-C ring3 entry: rip={USER_CODE_PAGE:#x} rsp={initial_user_rsp:#x} cs={user_cs:#x} ss={user_ss:#x} if=off return_vector={RETURN_VECTOR:#x}\n",
    ));

    // SAFETY: Both user pages are live in the current page table, selectors are
    // loaded DPL3 descriptors from M5-B, IF is clear, and RETURN_VECTOR is a
    // CI-only DPL3 interrupt gate that restores this kernel call frame.
    let returned = unsafe {
        __freeworld_m5c_enter_user(
            USER_CODE_PAGE,
            initial_user_rsp,
            user_cs,
            user_ss,
        )
    };
    assert_eq!(returned, 1);
    assert!(
        RETURN_SEEN.load(Ordering::Acquire),
        "M5-C did not return through the CPL3 gate"
    );
    assert!(
        !ARMED.load(Ordering::Acquire),
        "M5-C return gate did not consume its one-shot arm"
    );
    assert!(
        !super::interrupts_enabled(),
        "M5-C unexpectedly enabled interrupts during round-trip"
    );
    assert!(
        !interrupt_context::in_interrupt(),
        "M5-C returned with interrupt depth still active"
    );
    assert_eq!(
        SS::get_reg(),
        super::gdt::kernel_data_selector(),
        "M5-C did not restore the kernel data SS"
    );

    let observed_frame = OBSERVED_FRAME_ADDRESS.load(Ordering::Acquire);
    assert_ne!(observed_frame, 0);

    let unmapped_stack = memory::unmap_page(USER_STACK_PAGE)
        .expect("M5-C failed to unmap user stack");
    let unmapped_code = memory::unmap_page(USER_CODE_PAGE)
        .expect("M5-C failed to unmap user code");
    assert_eq!(unmapped_stack, stack_frame);
    assert_eq!(unmapped_code, code_frame);

    // SAFETY: Both user mappings are gone; the test retains exclusive
    // ownership of the allocator frames.
    unsafe {
        memory::free_frame(stack_frame)
            .expect("M5-C failed to recycle user stack frame");
        memory::free_frame(code_frame)
            .expect("M5-C failed to recycle user code frame");
    }

    serial::write_fmt(format_args!(
        "FreeWorldOS: M5-C ring3 return: frame={observed_frame:#x} rsp0_stack=ok user_rip=ok user_rsp=ok user_stack_marker=ok cpl3=observed cpl0=restored kernel_ss=restored frames_recycled=ok\n",
    ));
    serial::println(
        "FreeWorldOS: M5-C privilege round-trip: passed iretq_to_ring3=ok user_rx=ok user_rw_nx=ok dpl3_int_gate=test_only tss_rsp0=used ring0_return=ok scheduler=off user_task=off callgate=off",
    );
}

#[unsafe(no_mangle)]
extern "C" fn __freeworld_m5c_return_dispatch(frame: *mut UserReturnFrame) {
    let scope = interrupt_context::enter();

    assert!(
        ARMED.swap(false, Ordering::AcqRel),
        "unexpected or repeated M5-C return interrupt"
    );

    let frame_address = frame as u64;
    let rsp0_top = super::ring0_privilege_stack_top();
    let rsp0_bottom =
        rsp0_top - super::gdt::RING0_PRIVILEGE_STACK_SIZE as u64;

    assert!(
        frame_address >= rsp0_bottom
            && frame_address + USER_RETURN_FRAME_BYTES as u64 <= rsp0_top,
        "M5-C return frame did not land on TSS RSP0 stack"
    );

    // SAFETY: the hand-written gate pushed the same 15 GPRs used by the timer
    // frame on top of the hardware privilege-transition frame.
    let frame = unsafe { &*frame };

    let expected_rip = EXPECTED_USER_RIP.load(Ordering::Acquire);
    let expected_rsp = EXPECTED_USER_RSP.load(Ordering::Acquire);
    let user_cs = u64::from(super::user_code_selector().0);
    let user_ss = u64::from(super::user_data_selector().0);

    assert_eq!(frame.rip, expected_rip);
    assert_eq!(frame.rsp, expected_rsp);
    assert_eq!(frame.cs, user_cs);
    assert_eq!(frame.ss, user_ss);
    assert_eq!(frame.cs & 0x3, 0x3);
    assert_eq!(frame.ss & 0x3, 0x3);
    assert_eq!(frame.rflags & (1 << 9), 0);
    assert_eq!(frame.rax, USER_MARKER);
    assert_eq!(
        CS::get_reg().0 & 0x3,
        0,
        "M5-C return handler did not execute at CPL0"
    );

    // SAFETY: frame.rsp is the live mapped U+RW user stack and points at the
    // marker pushed by the CPL3 stub immediately before INT 0xF1.
    let marker = unsafe { core::ptr::read_volatile(frame.rsp as *const u64) };
    assert_eq!(marker, USER_MARKER);

    super::gdt::restore_kernel_data_segments();
    assert_eq!(
        SS::get_reg(),
        super::gdt::kernel_data_selector(),
        "M5-C return gate failed to restore kernel SS"
    );

    OBSERVED_FRAME_ADDRESS.store(frame_address, Ordering::Release);
    RETURN_SEEN.store(true, Ordering::Release);

    serial::write_fmt(format_args!(
        "FreeWorldOS: M5-C gate: vector={RETURN_VECTOR:#x} frame={frame_address:#x} user_cs={:#x} user_ss={:#x} user_rip={:#x} user_rsp={:#x} rsp0=ok marker=ok\n",
        frame.cs,
        frame.ss,
        frame.rip,
        frame.rsp,
    ));

    drop(scope);
}

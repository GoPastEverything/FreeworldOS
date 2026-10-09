// M5-G: one-shot, test-only entry into CPL3 using a ProcessObject PML4.
// No task scheduling, syscall ABI, or process RSP0 switching.
use core::{
    arch::global_asm,
    mem::size_of,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use x86_64::{
    registers::{
        control::Cr3,
        segmentation::{Segment, CS, SS},
    },
    VirtAddr,
};

use super::{gdt, interrupt_context, serial};

pub const RETURN_VECTOR: u8 = 0xF2;
pub const CODE_SIZE: usize = 14;
pub const USER_MARKER: u64 = 0x4d35_4752_3343_5033;

pub const fn stub_bytes() -> [u8; CODE_SIZE] {
    [
        0x48, 0xb8,
        (USER_MARKER >> 0) as u8,
        (USER_MARKER >> 8) as u8,
        (USER_MARKER >> 16) as u8,
        (USER_MARKER >> 24) as u8,
        (USER_MARKER >> 32) as u8,
        (USER_MARKER >> 40) as u8,
        (USER_MARKER >> 48) as u8,
        (USER_MARKER >> 56) as u8,
        0xcd, RETURN_VECTOR,
        0x0f, 0x0b,
    ]
}

const RETURN_RIP_OFFSET: u64 = 12;

#[repr(C)]
struct UserFrame {
    r15: u64, r14: u64, r13: u64, r12: u64,
    r11: u64, r10: u64, r9: u64, r8: u64,
    rbp: u64, rdi: u64, rsi: u64, rdx: u64,
    rcx: u64, rbx: u64, rax: u64,
    rip: u64, cs: u64, rflags: u64, rsp: u64, ss: u64,
}
const FRAME_SIZE: usize = size_of::<UserFrame>();
const _: () = assert!(FRAME_SIZE == 160);

static ARMED: AtomicBool = AtomicBool::new(false);
static RETURN_SEEN: AtomicBool = AtomicBool::new(false);
static EXPECTED_ROOT: AtomicU64 = AtomicU64::new(0);
static EXPECTED_RIP: AtomicU64 = AtomicU64::new(0);
static EXPECTED_RSP: AtomicU64 = AtomicU64::new(0);
static FRAME_ADDRESS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ring3ProcessProof {
    pub original_kernel_cr3: u64,
    pub restored_kernel_cr3: u64,
    pub observed_process_cr3: u64,
    pub frame_address: u64,
}

// Only one invocation at boot. These locations live in higher-half kernel
// storage copied into the process PML4; no concurrent user task can touch them.
global_asm!(
    r#"
    .intel_syntax noprefix
    .section .bss
    .align 8
__freeworld_m5g_saved_rsp:
    .quad 0
__freeworld_m5g_kernel_cr3:
    .quad 0
__freeworld_m5g_process_cr3_observed:
    .quad 0

    .section .text
    .global __freeworld_m5g_enter
    .type __freeworld_m5g_enter,@function
__freeworld_m5g_enter:
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15

    mov qword ptr [rip + __freeworld_m5g_saved_rsp], rsp
    mov rax, cr3
    mov qword ptr [rip + __freeworld_m5g_kernel_cr3], rax
    mov cr3, rdi
    push r8
    push rdx
    push 2
    push rcx
    push rsi
    iretq

__freeworld_m5g_resume:
    mov eax, 1
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx
    ret

    .global __freeworld_m5g_return_entry
    .type __freeworld_m5g_return_entry,@function
__freeworld_m5g_return_entry:
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

    mov rax, cr3
    mov qword ptr [rip + __freeworld_m5g_process_cr3_observed], rax
    mov rax, qword ptr [rip + __freeworld_m5g_kernel_cr3]
    mov cr3, rax

    mov rdi, rsp
    cld
    call __freeworld_m5g_return_dispatch
    mov rsp, qword ptr [rip + __freeworld_m5g_saved_rsp]
    jmp __freeworld_m5g_resume
    "#
);

unsafe extern "C" {
    fn __freeworld_m5g_enter(
        root_phys: u64, user_rip: u64, user_rsp: u64,
        user_cs: u64, user_ss: u64,
    ) -> u64;
    fn __freeworld_m5g_return_entry();
    static __freeworld_m5g_kernel_cr3: u64;
    static __freeworld_m5g_process_cr3_observed: u64;
}

pub fn handler_addr() -> VirtAddr {
    VirtAddr::new(__freeworld_m5g_return_entry as usize as u64)
}

// # Safety
// The caller must hold the process PML4 and U+RX leaf alive until this
// returns, with the expected user stub installed and the higher-half kernel
// mappings shared. There may be no other user-mode entry or scheduler handoff.
pub unsafe fn enter_once(root_phys: u64, user_rip: u64, user_rsp: u64) -> Ring3ProcessProof {
    assert!(!super::in_interrupt(), "M5-G called from interrupt handler");
    EXPECTED_ROOT.store(root_phys, Ordering::Release);
    EXPECTED_RIP.store(user_rip + RETURN_RIP_OFFSET, Ordering::Release);
    EXPECTED_RSP.store(user_rsp, Ordering::Release);
    FRAME_ADDRESS.store(0, Ordering::Release);
    RETURN_SEEN.store(false, Ordering::Release);
    assert!(!ARMED.swap(true, Ordering::AcqRel), "M5-G already armed");

    // Both CR3 writes and the CPL3 interval occur under IF=0.
    x86_64::instructions::interrupts::without_interrupts(|| {
        let (before, _) = Cr3::read();
        let original_kernel_cr3 = before.start_address().as_u64();
        assert_ne!(original_kernel_cr3, root_phys);

        let user_cs = u64::from(super::user_code_selector().0);
        let user_ss = u64::from(super::user_data_selector().0);
        let returned = unsafe {
            __freeworld_m5g_enter(root_phys, user_rip, user_rsp, user_cs, user_ss)
        };
        assert_eq!(returned, 1);
        assert!(RETURN_SEEN.load(Ordering::Acquire));
        assert!(!ARMED.load(Ordering::Acquire));
        assert!(!super::interrupts_enabled());
        assert!(!super::in_interrupt());
        assert_eq!(SS::get_reg(), gdt::kernel_data_selector());

        let (after, _) = Cr3::read();
        let restored_kernel_cr3 = after.start_address().as_u64();
        let observed_process_cr3 =
            unsafe { core::ptr::read_volatile(core::ptr::addr_of!(__freeworld_m5g_process_cr3_observed)) };
        let saved_kernel_cr3 =
            unsafe { core::ptr::read_volatile(core::ptr::addr_of!(__freeworld_m5g_kernel_cr3)) };

        assert_eq!(saved_kernel_cr3 & !0xfff, original_kernel_cr3);
        assert_eq!(restored_kernel_cr3, original_kernel_cr3);
        assert_eq!(observed_process_cr3 & !0xfff, root_phys);

        Ring3ProcessProof {
            original_kernel_cr3,
            restored_kernel_cr3,
            observed_process_cr3,
            frame_address: FRAME_ADDRESS.load(Ordering::Acquire),
        }
    })
}

#[unsafe(no_mangle)]
extern "C" fn __freeworld_m5g_return_dispatch(frame: *const UserFrame) {
    // Assembly has already restored kernel CR3 before entering this Rust code.
    let (current, _) = Cr3::read();
    let saved_cr3 =
        unsafe { core::ptr::read_volatile(core::ptr::addr_of!(__freeworld_m5g_kernel_cr3)) };
    assert_eq!(current.start_address().as_u64(), saved_cr3 & !0xfff);

    let scope = interrupt_context::enter();
    assert!(ARMED.swap(false, Ordering::AcqRel), "M5-G return not armed");

    let frame_address = frame as u64;
    let top = super::ring0_privilege_stack_top();
    let bottom = top - gdt::RING0_PRIVILEGE_STACK_SIZE as u64;
    assert!(frame_address >= bottom && frame_address + FRAME_SIZE as u64 <= top);

    let frame = unsafe { &*frame };
    assert_eq!(frame.cs, u64::from(super::user_code_selector().0));
    assert_eq!(frame.ss, u64::from(super::user_data_selector().0));
    assert_eq!(frame.cs & 3, 3);
    assert_eq!(frame.ss & 3, 3);
    assert_eq!(frame.rflags & (1 << 9), 0);
    assert_eq!(frame.rip, EXPECTED_RIP.load(Ordering::Acquire));
    assert_eq!(frame.rsp, EXPECTED_RSP.load(Ordering::Acquire));
    assert_eq!(frame.rax, USER_MARKER);
    assert_eq!(CS::get_reg().0 & 3, 0);

    let process_cr3 =
        unsafe { core::ptr::read_volatile(core::ptr::addr_of!(__freeworld_m5g_process_cr3_observed)) };
    assert_eq!(process_cr3 & !0xfff, EXPECTED_ROOT.load(Ordering::Acquire));

    gdt::restore_kernel_data_segments();
    assert_eq!(SS::get_reg(), gdt::kernel_data_selector());

    FRAME_ADDRESS.store(frame_address, Ordering::Release);
    RETURN_SEEN.store(true, Ordering::Release);

    serial::write_fmt(format_args!(
        "FreeWorldOS: M5-G return gate: vector={RETURN_VECTOR:#x} process_cr3={:#x} user_cs={:#x} user_ss={:#x} user_rip={:#x} user_rsp={:#x} rsp0=ok kernel_cr3=restored marker=ok\n",
        process_cr3 & !0xfff, frame.cs, frame.ss, frame.rip, frame.rsp,
    ));

    drop(scope);
}

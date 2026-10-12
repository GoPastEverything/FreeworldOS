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
#[cfg(feature = "m5o-ci-self-test")]
pub const SYSCALL_STUB: [u8; 16] = [
    0x48, 0xb8,
    (USER_MARKER >> 0) as u8, (USER_MARKER >> 8) as u8,
    (USER_MARKER >> 16) as u8, (USER_MARKER >> 24) as u8,
    (USER_MARKER >> 32) as u8, (USER_MARKER >> 40) as u8,
    (USER_MARKER >> 48) as u8, (USER_MARKER >> 56) as u8,
    0x0f, 0x05, // real SYSCALL; returns via SYSRETQ to offset 12
    0xcd, 0xf2, // test-only final exit, not the syscall mechanism
    0x0f, 0x0b,
];

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
const PUSH_RETURN_RIP_OFFSET: u64 = 13;
#[cfg(feature = "m5k-ci-self-test")]
const SUSPEND_RIP: u64 = 0x0000_5000_3000_000d;
#[cfg(feature = "m5k-ci-self-test")]
const FINAL_RIP: u64 = 0x0000_5000_3000_000f;
#[cfg(feature = "m5k-ci-self-test")]
pub const RESUME_CODE_SIZE: usize = 17;
#[cfg(feature = "m5k-ci-self-test")]
pub const fn resume_stub_bytes() -> [u8; RESUME_CODE_SIZE] {
    [
        0x48, 0xb8,
        (USER_MARKER >> 0) as u8, (USER_MARKER >> 8) as u8,
        (USER_MARKER >> 16) as u8, (USER_MARKER >> 24) as u8,
        (USER_MARKER >> 32) as u8, (USER_MARKER >> 40) as u8,
        (USER_MARKER >> 48) as u8, (USER_MARKER >> 56) as u8,
        0x50, 0xcd, RETURN_VECTOR, 0xcd, RETURN_VECTOR, 0x0f, 0x0b,
    ]
}
pub const PUSH_CODE_SIZE: usize = 15;
pub const fn push_stub_bytes() -> [u8; PUSH_CODE_SIZE] {
    [
        0x48, 0xb8,
        (USER_MARKER >> 0) as u8, (USER_MARKER >> 8) as u8,
        (USER_MARKER >> 16) as u8, (USER_MARKER >> 24) as u8,
        (USER_MARKER >> 32) as u8, (USER_MARKER >> 40) as u8,
        (USER_MARKER >> 48) as u8, (USER_MARKER >> 56) as u8,
        0x50, 0xcd, RETURN_VECTOR, 0x0f, 0x0b,
    ]
}


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
#[cfg(feature = "m5o-ci-self-test")]
static M5O_MODE: AtomicBool = AtomicBool::new(false);
static PUSH_MODE: AtomicBool = AtomicBool::new(false);
static RETURN_SEEN: AtomicBool = AtomicBool::new(false);
static EXPECTED_ROOT: AtomicU64 = AtomicU64::new(0);
static EXPECTED_RIP: AtomicU64 = AtomicU64::new(0);
static EXPECTED_RSP: AtomicU64 = AtomicU64::new(0);
static FRAME_ADDRESS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m5m-ci-self-test")]
static QUEUE_MODE: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m5k-ci-self-test")]
static SUSPEND_MODE: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m5k-ci-self-test")]
static TRAP_COUNT: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m5k-ci-self-test")]
static FIRST_FRAME: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m5k-ci-self-test")]
static FINAL_FRAME: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m5i-ci-self-test")]
static PRIVATE_STACK_BOTTOM: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m5i-ci-self-test")]
static PRIVATE_STACK_TOP: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m5i-ci-self-test")]
static PRIVATE_STACK_MODE: AtomicBool = AtomicBool::new(false);

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

    .global __freeworld_m5k_resume_frame
    .type __freeworld_m5k_resume_frame,@function
__freeworld_m5k_resume_frame:
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15
    mov qword ptr [rip + __freeworld_m5g_saved_rsp], rsp
    mov cr3, rsi
    mov rsp, rdi
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax
    iretq
    ud2

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


#[cfg(feature = "m5o-ci-self-test")]
global_asm!(r#"
    .intel_syntax noprefix
    .section .bss
    .align 8
__freeworld_m5o_user_rsp: .quad 0
__freeworld_m5o_kernel_top: .quad 0
__freeworld_m5o_seen: .quad 0
__freeworld_m5o_return_rcx: .quad 0
__freeworld_m5o_return_r11: .quad 0
__freeworld_m5o_observed_cr3: .quad 0
__freeworld_m5o_observed_rax: .quad 0

    .section .text
    .global __freeworld_m5o_syscall_entry
    .type __freeworld_m5o_syscall_entry,@function
__freeworld_m5o_syscall_entry:
    // SYSCALL does not switch RSP or CR3. Do not touch the user stack.
    mov qword ptr [rip + __freeworld_m5o_user_rsp], rsp
    mov rsp, qword ptr [rip + __freeworld_m5o_kernel_top]
    mov qword ptr [rip + __freeworld_m5o_return_rcx], rcx
    mov qword ptr [rip + __freeworld_m5o_return_r11], r11
    mov qword ptr [rip + __freeworld_m5o_observed_rax], rax
    mov rax, cr3
    mov qword ptr [rip + __freeworld_m5o_observed_cr3], rax
    mov qword ptr [rip + __freeworld_m5o_seen], 1
    mov rax, qword ptr [rip + __freeworld_m5o_observed_rax]
    mov rcx, qword ptr [rip + __freeworld_m5o_return_rcx]
    mov r11, qword ptr [rip + __freeworld_m5o_return_r11]
    mov rsp, qword ptr [rip + __freeworld_m5o_user_rsp]
    sysretq
    ud2
"#);

#[cfg(feature = "m5o-ci-self-test")]
unsafe extern "C" {
    fn __freeworld_m5o_syscall_entry();
    static __freeworld_m5o_user_rsp: u64;
    static __freeworld_m5o_kernel_top: u64;
    static __freeworld_m5o_seen: u64;
    static __freeworld_m5o_return_rcx: u64;
    static __freeworld_m5o_observed_cr3: u64;
    static __freeworld_m5o_observed_rax: u64;
}

#[cfg(feature = "m5o-ci-self-test")]
fn m5o_read(address: *const u64) -> u64 {
    unsafe { core::ptr::read_volatile(address) }
}

#[cfg(feature = "m5o-ci-self-test")]
fn m5o_write(address: *mut u64, value: u64) {
    unsafe { core::ptr::write_volatile(address, value) }
}

/// Test-only MSR setup, one CPU. No syscall ABI or scheduler dispatch.
#[cfg(feature = "m5o-ci-self-test")]
unsafe fn configure_m5o_syscall() {

    const STAR: u32 = 0xc000_0081;
    const LSTAR: u32 = 0xc000_0082;
    const SFMASK: u32 = 0xc000_0084;
    unsafe fn wrmsr(msr: u32, value: u64) {
        unsafe {
            core::arch::asm!("wrmsr", in("ecx") msr,
                in("eax") value as u32, in("edx") (value >> 32) as u32,
                options(nostack, preserves_flags));
        }
    }
    // Long-mode SYSRET derives ring-3 CS = STAR[63:48] + 16, SS = +8.
    let kcs = u64::from(CS::get_reg().0);
    assert_eq!(kcs & 3, 0);
    let ucs = u64::from(super::user_code_selector().0);
    let uds = u64::from(super::user_data_selector().0);
    assert_eq!(ucs, (uds + 8));
    let base = ucs - 16;
    let star = (base << 48) | (kcs << 32);
    unsafe {
        wrmsr(STAR, star);
        wrmsr(LSTAR, __freeworld_m5o_syscall_entry as usize as u64);
        // Mask IF, DF and TF on syscall entry. We only support IF=0 here.
        wrmsr(SFMASK, (1 << 9) | (1 << 10) | (1 << 8));
        let mut low: u32;
        let mut high: u32;
        core::arch::asm!("rdmsr", in("ecx") 0xc000_0080_u32,
            out("eax") low, out("edx") high, options(nostack, preserves_flags));
        wrmsr(0xc000_0080, (((high as u64) << 32) | low as u64) | 1);
    }
}

/// Enter one process through M5-G IRETQ, perform a genuine SYSCALL and
/// SYSRETQ, then exit through a *separate* test-only INT F2.
#[cfg(feature = "m5o-ci-self-test")]
pub unsafe fn ci_syscall_sysret(root: u64, bottom: u64, top: u64) -> Ring3ProcessProof {
    const CODE: u64 = 0x0000_5000_3000_0000;
    const USER_TOP: u64 = 0x0000_5000_3000_2000;
    assert_eq!(gdt::read_rsp0(), super::ring0_privilege_stack_top());
    assert_eq!(top - bottom, gdt::RING0_PRIVILEGE_STACK_SIZE as u64);
    x86_64::instructions::interrupts::without_interrupts(|| {
        assert!(!super::interrupts_enabled());
        unsafe { configure_m5o_syscall(); }
        m5o_write(core::ptr::addr_of!(__freeworld_m5o_kernel_top) as *mut u64, top);
        m5o_write(core::ptr::addr_of!(__freeworld_m5o_seen) as *mut u64, 0);
        let previous_rsp0 = gdt::read_rsp0();
        unsafe { gdt::write_rsp0(top); }
        PRIVATE_STACK_BOTTOM.store(bottom, Ordering::Release);
        PRIVATE_STACK_TOP.store(top, Ordering::Release);
        PRIVATE_STACK_MODE.store(true, Ordering::Release);
        M5O_MODE.store(true, Ordering::Release);
        let proof = unsafe { enter_mode(root, CODE, USER_TOP, false) };
        M5O_MODE.store(false, Ordering::Release);
        assert_eq!(proof.frame_address, top - FRAME_SIZE as u64);
        assert_eq!(m5o_read(core::ptr::addr_of!(__freeworld_m5o_seen)), 1);
        assert_eq!(m5o_read(core::ptr::addr_of!(__freeworld_m5o_user_rsp)), USER_TOP);
        assert_eq!(m5o_read(core::ptr::addr_of!(__freeworld_m5o_return_rcx)), CODE + 12);
        assert_eq!(m5o_read(core::ptr::addr_of!(__freeworld_m5o_observed_rax)), USER_MARKER);
        assert_eq!(m5o_read(core::ptr::addr_of!(__freeworld_m5o_observed_cr3)) & !0xfff, root);
        assert_eq!(gdt::read_rsp0(), top);
        unsafe { gdt::write_rsp0(previous_rsp0); }
        PRIVATE_STACK_MODE.store(false, Ordering::Release);
        assert_eq!(gdt::read_rsp0(), previous_rsp0);
        serial::write_fmt(format_args!(
            "FreeWorldOS: M5-O syscall/sysret: root={root:#x} syscall_rip={:#x} final_rip={:#x} user_rsp={USER_TOP:#x} frame={:#x} bootstrap_rsp0=restored\\n",
            CODE + 12, CODE + 14, proof.frame_address,
        ));
        proof
    })
}

unsafe extern "C" {
    fn __freeworld_m5g_enter(
        root_phys: u64, user_rip: u64, user_rsp: u64,
        user_cs: u64, user_ss: u64,
    ) -> u64;
    fn __freeworld_m5g_return_entry();
    #[cfg(feature = "m5k-ci-self-test")]
    fn __freeworld_m5k_resume_frame(frame: u64, process_cr3: u64) -> u64;
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
    unsafe { enter_mode(root_phys, user_rip, user_rsp, false) }
}

#[cfg(feature = "m5h-ci-self-test")]
pub unsafe fn enter_once_push(root_phys: u64, user_rip: u64, user_rsp: u64) -> Ring3ProcessProof {
    assert_eq!(user_rip, 0x0000_5000_3000_0000);
    assert_eq!(user_rsp, 0x0000_5000_3000_2000);
    unsafe { enter_mode(root_phys, user_rip, user_rsp, true) }
}

#[cfg(feature = "m5i-ci-self-test")]
pub unsafe fn enter_once_private_rsp0(
    root_phys: u64, user_rip: u64, user_rsp: u64, bottom: u64, top: u64
) -> Ring3ProcessProof {
    unsafe { enter_private_rsp0_from(root_phys, user_rip, user_rsp, bottom, top, super::ring0_privilege_stack_top()) }
}

#[cfg(feature = "m5l-ci-self-test")]
pub unsafe fn enter_private_rsp0_as_b(
    root_phys: u64, user_rip: u64, user_rsp: u64, bottom: u64, top: u64,
    a_rsp0: u64,
) -> Ring3ProcessProof {
    unsafe { enter_private_rsp0_from(root_phys, user_rip, user_rsp, bottom, top, a_rsp0) }
}

#[cfg(feature = "m5i-ci-self-test")]
unsafe fn enter_private_rsp0_from(
    root_phys: u64, user_rip: u64, user_rsp: u64, bottom: u64, top: u64,
    expected_prior_rsp0: u64,
) -> Ring3ProcessProof {
    assert_eq!(top - bottom, gdt::RING0_PRIVILEGE_STACK_SIZE as u64);
    assert_eq!(top & 0xf, 0);
    assert!(bottom >= 0xffff_8000_0000_0000);
    assert_eq!(gdt::read_rsp0(), expected_prior_rsp0);
    assert_ne!(top, expected_prior_rsp0, "M5-L B cannot share A's stack top");
    PRIVATE_STACK_BOTTOM.store(bottom, Ordering::Release);
    PRIVATE_STACK_TOP.store(top, Ordering::Release);
    PRIVATE_STACK_MODE.store(true, Ordering::Release);
    x86_64::instructions::interrupts::without_interrupts(|| {
        let saved = gdt::read_rsp0();
        unsafe { gdt::write_rsp0(top); }
        assert_eq!(gdt::read_rsp0(), top);
        let proof = unsafe { enter_mode(root_phys, user_rip, user_rsp, true) };
        unsafe { gdt::write_rsp0(saved); }
        assert_eq!(gdt::read_rsp0(), saved);
        assert!(proof.frame_address >= bottom && proof.frame_address + FRAME_SIZE as u64 <= top);
        PRIVATE_STACK_MODE.store(false, Ordering::Release);
        proof
    })
}

#[cfg(feature = "m5k-ci-self-test")]
pub unsafe fn suspend_and_resume_once(
    root_phys: u64, user_rip: u64, user_rsp: u64, bottom: u64, top: u64
) -> Ring3ProcessProof {
    unsafe { suspend_resume_with_hook(root_phys, user_rip, user_rsp, bottom, top, || {}) }
}

/// M5-L: the hook executes on the bootstrap kernel call stack after A's
/// first full interrupt frame is saved, and before its original IRETQ.
#[cfg(feature = "m5l-ci-self-test")]
pub unsafe fn suspend_a_run_b_resume_a(
    root_phys: u64, user_rip: u64, user_rsp: u64, bottom: u64, top: u64,
    run_b: impl FnOnce(),
) -> Ring3ProcessProof {
    unsafe { suspend_resume_with_hook(root_phys, user_rip, user_rsp, bottom, top, run_b) }
}

pub(crate) unsafe fn suspend_resume_with_hook(
    root_phys: u64, user_rip: u64, user_rsp: u64, bottom: u64, top: u64,
    run_during_suspend: impl FnOnce(),
) -> Ring3ProcessProof {
    assert_eq!(user_rip, 0x0000_5000_3000_0000);
    assert_eq!(user_rsp, 0x0000_5000_3000_2000);
    assert_eq!(top - bottom, gdt::RING0_PRIVILEGE_STACK_SIZE as u64);
    assert_eq!(top & 15, 0);
    assert_eq!(gdt::read_rsp0(), super::ring0_privilege_stack_top());
    assert!(!super::in_interrupt());
    PRIVATE_STACK_BOTTOM.store(bottom, Ordering::Release);
    PRIVATE_STACK_TOP.store(top, Ordering::Release);
    PRIVATE_STACK_MODE.store(true, Ordering::Release);
    SUSPEND_MODE.store(true, Ordering::Release);
    TRAP_COUNT.store(0, Ordering::Release);
    FIRST_FRAME.store(0, Ordering::Release);
    FINAL_FRAME.store(0, Ordering::Release);
    // Both transitions remain masked. The initial gate saves bootstrap CR3
    // and the kernel call-stack continuation. The resumption gate preserves
    // the same 160-byte hardware frame, not the stub entry point.
    x86_64::instructions::interrupts::without_interrupts(|| {
        assert!(!super::interrupts_enabled());
        let bootstrap_rsp0 = gdt::read_rsp0();
        unsafe { gdt::write_rsp0(top); }
        let first = unsafe { enter_mode(root_phys, user_rip, user_rsp, true) };
        assert_eq!(TRAP_COUNT.load(Ordering::Acquire), 1);
        assert_eq!(first.frame_address, top - FRAME_SIZE as u64);
        assert_eq!(FIRST_FRAME.load(Ordering::Acquire), first.frame_address);
        assert_eq!(gdt::read_rsp0(), top, "RSP0 restored during suspension");
        // Resume target is the second INT, not the original MOV/PUSH.
        let suspended = unsafe { &*(first.frame_address as *const UserFrame) };
        assert_eq!(suspended.rip, SUSPEND_RIP);
        assert_eq!(suspended.rsp, user_rsp - 8);
        assert_eq!(suspended.rax, USER_MARKER);
        assert_eq!(suspended.rflags & (1 << 9), 0);
        assert_eq!(suspended.cs & 3, 3);
        assert_eq!(suspended.ss & 3, 3);
        // Preserve every byte of A's 160-byte saved context while another
        // independent process B enters Ring 3 and returns.
        let first_words: [u64; 20] = core::array::from_fn(|i| unsafe {
            core::ptr::read_volatile((first.frame_address as *const u64).add(i))
        });
        #[cfg(feature = "m5l-ci-self-test")]
        {
            // B is not a continuation of A. Disarm A's test-only trap count
            // while B uses the same 0xF2 IDT gate and shared instrumentation.
            SUSPEND_MODE.store(false, Ordering::Release);
            PRIVATE_STACK_MODE.store(false, Ordering::Release);
        }
        run_during_suspend();
        #[cfg(feature = "m5l-ci-self-test")]
        {
            PRIVATE_STACK_BOTTOM.store(bottom, Ordering::Release);
            PRIVATE_STACK_TOP.store(top, Ordering::Release);
            PRIVATE_STACK_MODE.store(true, Ordering::Release);
            SUSPEND_MODE.store(true, Ordering::Release);
            EXPECTED_ROOT.store(root_phys, Ordering::Release);
            EXPECTED_RSP.store(user_rsp - 8, Ordering::Release);
            PUSH_MODE.store(true, Ordering::Release);
            assert_eq!(TRAP_COUNT.load(Ordering::Acquire), 1);
        }
        for (i, original) in first_words.into_iter().enumerate() {
            let current = unsafe {
                core::ptr::read_volatile((first.frame_address as *const u64).add(i))
            };
            assert_eq!(current, original, "M5-L: B corrupted A's saved 160-byte context");
        }
        assert_eq!(gdt::read_rsp0(), top, "B did not restore A's private RSP0");
        EXPECTED_RIP.store(FINAL_RIP, Ordering::Release);
        RETURN_SEEN.store(false, Ordering::Release);
        assert!(!ARMED.swap(true, Ordering::AcqRel));
        unsafe { gdt::write_rsp0(top - FRAME_SIZE as u64); }
        assert_eq!(gdt::read_rsp0(), top - FRAME_SIZE as u64);
        let returned = unsafe { __freeworld_m5k_resume_frame(first.frame_address, root_phys) };
        assert_eq!(returned, 1);
        assert_eq!(TRAP_COUNT.load(Ordering::Acquire), 2);
        assert!(RETURN_SEEN.load(Ordering::Acquire));
        assert_eq!(FINAL_FRAME.load(Ordering::Acquire), top - 2 * FRAME_SIZE as u64);
        assert_eq!(FRAME_ADDRESS.load(Ordering::Acquire), top - 2 * FRAME_SIZE as u64);
        assert_eq!(gdt::read_rsp0(), top - FRAME_SIZE as u64, "RSP0 restored before final return");
        let (after, _) = Cr3::read();
        assert_eq!(after.start_address().as_u64(), first.original_kernel_cr3);
        unsafe { gdt::write_rsp0(bootstrap_rsp0); }
        assert_eq!(gdt::read_rsp0(), bootstrap_rsp0);
        assert!(!ARMED.load(Ordering::Acquire));
        assert!(!super::interrupts_enabled());
        PRIVATE_STACK_MODE.store(false, Ordering::Release);
        SUSPEND_MODE.store(false, Ordering::Release);
        serial::write_fmt(format_args!(
            "FreeWorldOS: M5-K two traps: first={:#x} second={:#x} first_rip={SUSPEND_RIP:#x} final_rip={FINAL_RIP:#x} rsp0=restored_after_final\\n",
            first.frame_address, FINAL_FRAME.load(Ordering::Acquire)
        ));
        Ring3ProcessProof {
            original_kernel_cr3: first.original_kernel_cr3,
            restored_kernel_cr3: after.start_address().as_u64(),
            observed_process_cr3: first.observed_process_cr3,
            frame_address: first.frame_address,
        }
    })
}

// M5-M only. The caller owns both processes and keeps their stack frames
// mapped until the entire explicit A,B,A pick sequence is complete.
#[cfg(feature = "m5m-ci-self-test")]
#[derive(Clone, Copy, Debug)]
pub struct ExplicitQueueProof {
    pub root_a: u64,
    pub root_b: u64,
    pub frame_a_first: u64,
    pub frame_a_second: u64,
    pub frame_a_final: u64,
    pub frame_b_first: u64,
    pub frame_b_second: u64,
}

#[cfg(feature = "m5m-ci-self-test")]
pub fn queue_stub_bytes(a: bool) -> [u8; 19] {
    // A: MOV/PUSH/INT/INT/INT/UD2. B: MOV/PUSH/INT/INT/UD2/NOP/NOP.
    let mut bytes = [0x90; 19];
    let source = resume_stub_bytes();
    bytes[..15].copy_from_slice(&source[..15]);
    if a {
        bytes[15] = 0xcd; bytes[16] = RETURN_VECTOR;
        bytes[17] = 0x0f; bytes[18] = 0x0b;
    } else {
        bytes[15] = 0x0f; bytes[16] = 0x0b;
    }
    bytes
}

#[cfg(feature = "m5m-ci-self-test")]
pub unsafe fn run_explicit_two_slot_queue(
    a_root: u64, a_bottom: u64, a_top: u64,
    b_root: u64, b_bottom: u64, b_top: u64,
) -> ExplicitQueueProof {
    use crate::object::explicit_queue::{Slot, TwoSlotQueue};
    const ENTRY: u64 = 0x0000_5000_3000_0000;
    const USER_TOP: u64 = 0x0000_5000_3000_2000;
    assert_ne!(a_root, b_root);
    assert!(a_top <= b_bottom || b_top <= a_bottom);
    assert_eq!(a_top - a_bottom, gdt::RING0_PRIVILEGE_STACK_SIZE as u64);
    assert_eq!(b_top - b_bottom, gdt::RING0_PRIVILEGE_STACK_SIZE as u64);
    assert_eq!(gdt::read_rsp0(), super::ring0_privilege_stack_top());
    assert!(!super::in_interrupt());

    x86_64::instructions::interrupts::without_interrupts(|| {
        let bootstrap = gdt::read_rsp0();
        QUEUE_MODE.store(true, Ordering::Release);
        let mut queue = TwoSlotQueue::new_suspended();

        // Both first interrupts complete before pick_next is ever called.
        unsafe { gdt::write_rsp0(a_top); }
        set_queue_stack(a_bottom, a_top);
        let a_initial = unsafe { enter_mode(a_root, ENTRY, USER_TOP, true) };
        let a_first = a_initial.frame_address;
        assert_eq!(a_first, a_top - FRAME_SIZE as u64);
        let a_snapshot = snapshot_frame(a_first);
        assert_eq!(gdt::read_rsp0(), a_top);

        unsafe { gdt::write_rsp0(b_top); }
        set_queue_stack(b_bottom, b_top);
        let b_initial = unsafe { enter_mode(b_root, ENTRY, USER_TOP, true) };
        let b_first = b_initial.frame_address;
        assert_eq!(b_first, b_top - FRAME_SIZE as u64);
        let b_snapshot = snapshot_frame(b_first);
        assert_eq!(gdt::read_rsp0(), b_top);
        check_frame_unchanged(a_first, &a_snapshot);

        assert_eq!(queue.pick_next().expect("M5-M pick A"), Slot::A);
        set_queue_stack(a_bottom, a_top);
        unsafe { gdt::write_rsp0(a_first); }
        let a_second = unsafe { resume_queue_frame(a_first, a_root, ENTRY + 15) };
        assert_eq!(a_second, a_top - 2 * FRAME_SIZE as u64);
        check_frame_unchanged(b_first, &b_snapshot);

        assert_eq!(queue.pick_next().expect("M5-M pick B"), Slot::B);
        set_queue_stack(b_bottom, b_top);
        unsafe { gdt::write_rsp0(b_first); }
        let b_second = unsafe { resume_queue_frame(b_first, b_root, ENTRY + 15) };
        assert_eq!(b_second, b_top - 2 * FRAME_SIZE as u64);
        let b_second_snapshot = snapshot_frame(b_second);
        let a_second_snapshot = snapshot_frame(a_second);

        assert_eq!(queue.pick_next().expect("M5-M pick A final"), Slot::A);
        check_frame_unchanged(a_first, &a_snapshot);
        check_frame_unchanged(a_second, &a_second_snapshot);
        check_frame_unchanged(b_first, &b_snapshot);
        check_frame_unchanged(b_second, &b_second_snapshot);
        set_queue_stack(a_bottom, a_top);
        unsafe { gdt::write_rsp0(a_second); }
        let a_final = unsafe { resume_queue_frame(a_second, a_root, ENTRY + 17) };
        assert_eq!(a_final, a_top - 3 * FRAME_SIZE as u64);
        assert_eq!(gdt::read_rsp0(), a_second);
        assert!(queue.pick_next().is_err(), "queue must not silently cycle");
        check_frame_unchanged(b_second, &b_second_snapshot);
        let (kernel, _) = Cr3::read();
        assert_eq!(kernel.start_address().as_u64(), a_initial.original_kernel_cr3);
        assert_eq!(kernel.start_address().as_u64(), b_initial.original_kernel_cr3);
        assert_ne!(gdt::read_rsp0(), bootstrap, "RSP0 restored before final return");
        unsafe { gdt::write_rsp0(bootstrap); }
        assert_eq!(gdt::read_rsp0(), bootstrap);
        QUEUE_MODE.store(false, Ordering::Release);
        PRIVATE_STACK_MODE.store(false, Ordering::Release);
        assert!(!super::interrupts_enabled());
        assert!(!ARMED.load(Ordering::Acquire));
        serial::write_fmt(format_args!(
            "FreeWorldOS: M5-M queue: picks=A,B,A root_a={a_root:#x} root_b={b_root:#x} a_first={a_first:#x} a_second={a_second:#x} a_final={a_final:#x} b_first={b_first:#x} b_second={b_second:#x} bootstrap_rsp0=restored_after_final\\n"
        ));
        ExplicitQueueProof {
            root_a: a_root, root_b: b_root,
            frame_a_first: a_first, frame_a_second: a_second,
            frame_a_final: a_final, frame_b_first: b_first,
            frame_b_second: b_second,
        }
    })
}

#[cfg(feature = "m5m-ci-self-test")]
fn snapshot_frame(address: u64) -> [u64; 20] {
    core::array::from_fn(|i| unsafe {
        core::ptr::read_volatile((address as *const u64).add(i))
    })
}

#[cfg(feature = "m5m-ci-self-test")]
fn check_frame_unchanged(address: u64, snapshot: &[u64; 20]) {
    for (i, original) in snapshot.iter().enumerate() {
        assert_eq!(unsafe { core::ptr::read_volatile((address as *const u64).add(i)) },
            *original, "M5-M: other process overwrote suspended frame");
    }
}

#[cfg(feature = "m5m-ci-self-test")]
fn set_queue_stack(bottom: u64, top: u64) {
    PRIVATE_STACK_BOTTOM.store(bottom, Ordering::Release);
    PRIVATE_STACK_TOP.store(top, Ordering::Release);
    PRIVATE_STACK_MODE.store(true, Ordering::Release);
}

#[cfg(feature = "m5m-ci-self-test")]
unsafe fn resume_queue_frame(frame: u64, root: u64, expected_rip: u64) -> u64 {
    assert_eq!(unsafe { (*(frame as *const UserFrame)).rip }, expected_rip - 2);
    EXPECTED_ROOT.store(root, Ordering::Release);
    EXPECTED_RSP.store(0x0000_5000_3000_1ff8, Ordering::Release);
    EXPECTED_RIP.store(expected_rip, Ordering::Release);
    FRAME_ADDRESS.store(0, Ordering::Release);
    RETURN_SEEN.store(false, Ordering::Release);
    assert!(!ARMED.swap(true, Ordering::AcqRel));
    let (kernel_before, _) = Cr3::read();
    let returned = unsafe { __freeworld_m5k_resume_frame(frame, root) };
    assert_eq!(returned, 1);
    assert!(RETURN_SEEN.load(Ordering::Acquire));
    assert!(!ARMED.load(Ordering::Acquire));
    assert_eq!(Cr3::read().0.start_address().as_u64(),
        kernel_before.start_address().as_u64());
    FRAME_ADDRESS.load(Ordering::Acquire)
}

/// M5-N: one process, one kernel-owned RX user alias. The CALL itself does
/// not switch rings. The gate's INT F2 performs the CPL3->CPL0 transition;
/// the original 160-byte frame is IRETQ-resumed to its RET instruction.
#[cfg(feature = "m5n-ci-self-test")]
pub unsafe fn ci_callgate_roundtrip(root: u64, bottom: u64, top: u64) -> Ring3ProcessProof {
    const CODE: u64 = 0x0000_5000_3000_0000;
    const GATE: u64 = CODE + 0x2000;
    const USER_RSP: u64 = CODE + 0x2000;
    assert_eq!(gdt::read_rsp0(), super::ring0_privilege_stack_top());
    assert_eq!(top - bottom, gdt::RING0_PRIVILEGE_STACK_SIZE as u64);
    assert!(!super::in_interrupt());
    x86_64::instructions::interrupts::without_interrupts(|| {
        let (kernel_before, _) = Cr3::read();
        let bootstrap = gdt::read_rsp0();
        PRIVATE_STACK_BOTTOM.store(bottom, Ordering::Release);
        PRIVATE_STACK_TOP.store(top, Ordering::Release);
        PRIVATE_STACK_MODE.store(true, Ordering::Release);
        EXPECTED_ROOT.store(root, Ordering::Release);
        EXPECTED_RIP.store(GATE + 2, Ordering::Release);
        EXPECTED_RSP.store(USER_RSP - 8, Ordering::Release);
        FRAME_ADDRESS.store(0, Ordering::Release);
        RETURN_SEEN.store(false, Ordering::Release);
        PUSH_MODE.store(false, Ordering::Release);
        assert!(!ARMED.swap(true, Ordering::AcqRel));
        unsafe { gdt::write_rsp0(top); }
        let returned = unsafe {
            __freeworld_m5g_enter(
                root, CODE, USER_RSP,
                u64::from(super::user_code_selector().0),
                u64::from(super::user_data_selector().0),
            )
        };
        assert_eq!(returned, 1);
        assert!(RETURN_SEEN.load(Ordering::Acquire));
        assert!(!ARMED.load(Ordering::Acquire));
        let first = FRAME_ADDRESS.load(Ordering::Acquire);
        assert_eq!(first, top - FRAME_SIZE as u64);
        let saved = unsafe { &*(first as *const UserFrame) };
        assert_eq!(saved.rip, GATE + 2);
        assert_eq!(saved.rsp, USER_RSP - 8);
        assert_eq!(gdt::read_rsp0(), top);
        assert_eq!(Cr3::read().0.start_address().as_u64(), kernel_before.start_address().as_u64());

        // IRETQ back to RET in the gate page, which pops the CALL return
        // address and reaches the next instruction with original user RSP.
        EXPECTED_RIP.store(CODE + 17, Ordering::Release);
        EXPECTED_RSP.store(USER_RSP, Ordering::Release);
        FRAME_ADDRESS.store(0, Ordering::Release);
        RETURN_SEEN.store(false, Ordering::Release);
        assert!(!ARMED.swap(true, Ordering::AcqRel));
        unsafe { gdt::write_rsp0(first); }
        assert_eq!(unsafe { __freeworld_m5k_resume_frame(first, root) }, 1);
        assert!(RETURN_SEEN.load(Ordering::Acquire));
        assert!(!ARMED.load(Ordering::Acquire));
        let second = FRAME_ADDRESS.load(Ordering::Acquire);
        assert_eq!(second, top - 2 * FRAME_SIZE as u64);
        let last = unsafe { &*(second as *const UserFrame) };
        assert_eq!(last.rip, CODE + 17);
        assert_eq!(last.rsp, USER_RSP);
        assert_eq!(gdt::read_rsp0(), first);
        assert_eq!(Cr3::read().0.start_address().as_u64(), kernel_before.start_address().as_u64());
        unsafe { gdt::write_rsp0(bootstrap); }
        assert_eq!(gdt::read_rsp0(), bootstrap);
        assert!(!super::interrupts_enabled());
        PRIVATE_STACK_MODE.store(false, Ordering::Release);
        serial::write_fmt(format_args!(
            "FreeWorldOS: M5-N callgate: gate={GATE:#x} first={first:#x} second={second:#x} gate_return={:#x} user_rsp={USER_RSP:#x} rsp0=restored\\n",
            CODE + 15,
        ));
        Ring3ProcessProof {
            original_kernel_cr3: kernel_before.start_address().as_u64(),
            restored_kernel_cr3: Cr3::read().0.start_address().as_u64(),
            observed_process_cr3: root,
            frame_address: first,
        }
    })
}

unsafe fn enter_mode(root_phys: u64, user_rip: u64, user_rsp: u64, push_mode: bool) -> Ring3ProcessProof {
    assert!(!super::in_interrupt(), "M5-G called from interrupt handler");
    EXPECTED_ROOT.store(root_phys, Ordering::Release);
    EXPECTED_RIP.store(user_rip + if push_mode { PUSH_RETURN_RIP_OFFSET } else {
        #[cfg(feature = "m5o-ci-self-test")]
        { if M5O_MODE.load(Ordering::Acquire) { 14 } else { RETURN_RIP_OFFSET } }
        #[cfg(not(feature = "m5o-ci-self-test"))]
        { RETURN_RIP_OFFSET }
    }, Ordering::Release);
    EXPECTED_RSP.store(user_rsp - if push_mode { 8 } else { 0 }, Ordering::Release);
    PUSH_MODE.store(push_mode, Ordering::Release);
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
        if push_mode {
            assert_eq!(EXPECTED_RIP.load(Ordering::Acquire), 0x0000_5000_3000_000d);
            assert_eq!(EXPECTED_RSP.load(Ordering::Acquire), 0x0000_5000_3000_1ff8);
        }

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
    #[cfg(feature = "m5k-ci-self-test")]
    let trap_index = if SUSPEND_MODE.load(Ordering::Acquire) {
        TRAP_COUNT.fetch_add(1, Ordering::AcqRel) + 1
    } else { 0 };

    let frame_address = frame as u64;
    let (bottom, top) = {
        #[cfg(feature = "m5i-ci-self-test")]
        {
            if PRIVATE_STACK_MODE.load(Ordering::Acquire) {
                (PRIVATE_STACK_BOTTOM.load(Ordering::Acquire), PRIVATE_STACK_TOP.load(Ordering::Acquire))
            } else {
                let top = super::ring0_privilege_stack_top();
                (top - gdt::RING0_PRIVILEGE_STACK_SIZE as u64, top)
            }
        }
        #[cfg(not(feature = "m5i-ci-self-test"))]
        {
            let top = super::ring0_privilege_stack_top();
            (top - gdt::RING0_PRIVILEGE_STACK_SIZE as u64, top)
        }
    };
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

    #[cfg(feature = "m5m-ci-self-test")]
    if QUEUE_MODE.load(Ordering::Acquire) {
        assert_eq!(frame_address + FRAME_SIZE as u64, gdt::read_rsp0());
        // CPU uses RSP0 as exclusive top, and the 15 saved GPRs precede
        // the hardware frame. The test owner must retain this stack.
    }
    #[cfg(feature = "m5k-ci-self-test")]
    if SUSPEND_MODE.load(Ordering::Acquire) {
        let top = PRIVATE_STACK_TOP.load(Ordering::Acquire);
        if trap_index == 1 {
            assert_eq!(frame_address, top - FRAME_SIZE as u64);
            assert_eq!(frame.rip, SUSPEND_RIP);
            FIRST_FRAME.store(frame_address, Ordering::Release);
        } else {
            assert_eq!(trap_index, 2);
            assert_eq!(frame_address, top - 2 * FRAME_SIZE as u64);
            assert_eq!(frame.rip, FINAL_RIP);
            assert_eq!(FIRST_FRAME.load(Ordering::Acquire), top - FRAME_SIZE as u64);
            FINAL_FRAME.store(frame_address, Ordering::Release);
        }
    }
    FRAME_ADDRESS.store(frame_address, Ordering::Release);
    RETURN_SEEN.store(true, Ordering::Release);

    #[cfg(feature = "m5i-ci-self-test")]
    if PRIVATE_STACK_MODE.load(Ordering::Acquire) {
        serial::write_fmt(format_args!(
            "FreeWorldOS: M5-I private RSP0 gate: frame={frame_address:#x} bottom={bottom:#x} top={top:#x} bytes=160 process_cr3={:#x} kernel_cr3=restored\\n",
            process_cr3 & !0xfff
        ));
        drop(scope);
        return;
    }
    if PUSH_MODE.load(Ordering::Acquire) {
        serial::write_fmt(format_args!(
            "FreeWorldOS: M5-H return gate: vector={RETURN_VECTOR:#x} process_cr3={:#x} user_cs={:#x} user_ss={:#x} user_rip={:#x} user_rsp={:#x} rsp0=ok kernel_cr3=restored marker=ok\n",
            process_cr3 & !0xfff, frame.cs, frame.ss, frame.rip, frame.rsp,
        ));
        return;
    }
    serial::write_fmt(format_args!(
        "FreeWorldOS: M5-G return gate: vector={RETURN_VECTOR:#x} process_cr3={:#x} user_cs={:#x} user_ss={:#x} user_rip={:#x} user_rsp={:#x} rsp0=ok kernel_cr3=restored marker=ok\n",
        process_cr3 & !0xfff, frame.cs, frame.ss, frame.rip, frame.rsp,
    ));

    drop(scope);
}

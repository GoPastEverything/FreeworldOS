use core::{
    arch::global_asm,
    mem::size_of,
};

use x86_64::VirtAddr;

use super::{apic, interrupt_context};

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TimerInterruptFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

pub const TIMER_INTERRUPT_FRAME_BYTES: usize = size_of::<TimerInterruptFrame>();

const _: () = {
    assert!(TIMER_INTERRUPT_FRAME_BYTES == 20 * size_of::<u64>());
    assert!(TIMER_INTERRUPT_FRAME_BYTES == 160);
};

global_asm!(
    r#"
    .intel_syntax noprefix
    .section .text

    .global __freeworld_apic_timer_entry
    .type __freeworld_apic_timer_entry,@function
__freeworld_apic_timer_entry:
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
    call __freeworld_apic_timer_dispatch

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

    .global __freeworld_probe_timer_callee_saved
    .type __freeworld_probe_timer_callee_saved,@function
__freeworld_probe_timer_callee_saved:
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15

    mov rbx, 0x11111111
    mov rbp, 0x22222222
    mov r12, 0x33333333
    mov r13, 0x44444444
    mov r14, 0x55555555
    mov r15, 0x66666666

    hlt

    mov eax, 1
    cmp rbx, 0x11111111
    jne 1f
    cmp rbp, 0x22222222
    jne 1f
    cmp r12, 0x33333333
    jne 1f
    cmp r13, 0x44444444
    jne 1f
    cmp r14, 0x55555555
    jne 1f
    cmp r15, 0x66666666
    jne 1f
    jmp 2f
1:
    xor eax, eax
2:
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx
    ret
    "#
);

unsafe extern "C" {
    fn __freeworld_apic_timer_entry();
    fn __freeworld_probe_timer_callee_saved() -> u64;
}

pub fn handler_addr() -> VirtAddr {
    VirtAddr::new(__freeworld_apic_timer_entry as usize as u64)
}

/// Runs one HLT while fixed values occupy the SysV callee-saved registers and
/// returns whether the timer entry restored every value.
///
/// # Safety
///
/// IF must be enabled and the periodic LAPIC timer must be running.
#[cfg(feature = "m35c2f-ci-trap-frame-test")]
pub unsafe fn probe_timer_callee_saved_once() -> bool {
    unsafe { __freeworld_probe_timer_callee_saved() != 0 }
}

#[unsafe(no_mangle)]
extern "C" fn __freeworld_apic_timer_dispatch(frame: *mut TimerInterruptFrame) {
    let frame_address = frame as u64;
    let aligned = frame_address & 0xf == 0;

    // SAFETY: The assembly entry passes RSP after all 15 GPR pushes. Intel 64
    // has already pushed SS, RSP, RFLAGS, CS and RIP, so this is the start of
    // the live 160-byte timer frame.
    let frame_ref = unsafe { &*frame };

    let scope = interrupt_context::enter();

    #[cfg(feature = "m35c2f-ci-trap-frame-test")]
    crate::rt::scheduler::timer_interrupt_frame_enter(
        frame_address,
        TIMER_INTERRUPT_FRAME_BYTES as u64,
        frame_ref.rsp,
        frame_ref.rflags,
        aligned,
    );

    // Tick accounting and LAPIC EOI happen here. Any future scheduling
    // decision from timer context must remain after this controller completion.
    apic::timer_interrupt();

    #[cfg(feature = "m35c2f-ci-trap-frame-test")]
    crate::rt::scheduler::timer_interrupt_frame_return_same_task();

    // Interrupt depth belongs to active handler execution, not to a saved task
    // frame. A future handoff must happen only after this scope is dropped.
    drop(scope);
}

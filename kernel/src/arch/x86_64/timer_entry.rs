use core::{
    arch::global_asm,
    mem::size_of,
};

use x86_64::{
    registers::segmentation::{Segment, CS},
    VirtAddr,
};

const PROBE_RAX: u64 = 0x1101;
const PROBE_RBX: u64 = 0x2202;
const PROBE_RCX: u64 = 0x3303;
const PROBE_RDX: u64 = 0x4404;
const PROBE_RSI: u64 = 0x5505;
const PROBE_RDI: u64 = 0x6606;
const PROBE_RBP: u64 = 0x7707;
const PROBE_R8: u64 = 0x0808;
const PROBE_R9: u64 = 0x0909;
const PROBE_R10: u64 = 0x1010;
const PROBE_R11: u64 = 0x1111;
const PROBE_R12: u64 = 0x1212;
const PROBE_R13: u64 = 0x1313;
const PROBE_R14: u64 = 0x1414;
const PROBE_R15: u64 = 0x1515;

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

    mov rdi, rsp
    jmp __freeworld_resume_interrupt_context

    .global __freeworld_probe_timer_all_gprs
    .type __freeworld_probe_timer_all_gprs,@function
__freeworld_probe_timer_all_gprs:
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15

    mov rax, {probe_rax}
    mov rbx, {probe_rbx}
    mov rcx, {probe_rcx}
    mov rdx, {probe_rdx}
    mov rsi, {probe_rsi}
    mov rdi, {probe_rdi}
    mov rbp, {probe_rbp}
    mov r8,  {probe_r8}
    mov r9,  {probe_r9}
    mov r10, {probe_r10}
    mov r11, {probe_r11}
    mov r12, {probe_r12}
    mov r13, {probe_r13}
    mov r14, {probe_r14}
    mov r15, {probe_r15}

    hlt
    .global __freeworld_probe_after_hlt
__freeworld_probe_after_hlt:
    cmp rax, {probe_rax}
    jne 1f
    cmp rbx, {probe_rbx}
    jne 1f
    cmp rcx, {probe_rcx}
    jne 1f
    cmp rdx, {probe_rdx}
    jne 1f
    cmp rsi, {probe_rsi}
    jne 1f
    cmp rdi, {probe_rdi}
    jne 1f
    cmp rbp, {probe_rbp}
    jne 1f
    cmp r8,  {probe_r8}
    jne 1f
    cmp r9,  {probe_r9}
    jne 1f
    cmp r10, {probe_r10}
    jne 1f
    cmp r11, {probe_r11}
    jne 1f
    cmp r12, {probe_r12}
    jne 1f
    cmp r13, {probe_r13}
    jne 1f
    cmp r14, {probe_r14}
    jne 1f
    cmp r15, {probe_r15}
    jne 1f

    mov eax, 1
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
    "#,
    probe_rax = const PROBE_RAX,
    probe_rbx = const PROBE_RBX,
    probe_rcx = const PROBE_RCX,
    probe_rdx = const PROBE_RDX,
    probe_rsi = const PROBE_RSI,
    probe_rdi = const PROBE_RDI,
    probe_rbp = const PROBE_RBP,
    probe_r8 = const PROBE_R8,
    probe_r9 = const PROBE_R9,
    probe_r10 = const PROBE_R10,
    probe_r11 = const PROBE_R11,
    probe_r12 = const PROBE_R12,
    probe_r13 = const PROBE_R13,
    probe_r14 = const PROBE_R14,
    probe_r15 = const PROBE_R15,
);

unsafe extern "C" {
    fn __freeworld_apic_timer_entry();
    fn __freeworld_probe_timer_all_gprs() -> u64;
    static __freeworld_probe_after_hlt: u8;
}

pub fn handler_addr() -> VirtAddr {
    VirtAddr::new(__freeworld_apic_timer_entry as usize as u64)
}

/// Runs one HLT while distinct fixed values occupy all fifteen GPRs and
/// returns whether the timer entry restored every value.
///
/// # Safety
///
/// IF must be enabled and the periodic LAPIC timer must be running.
#[cfg(any(
    feature = "m35c2f-ci-trap-frame-test",
    feature = "m35c2g-ci-resume-interrupt-test",
))]
pub unsafe fn probe_timer_all_gprs_once() -> bool {
    unsafe { __freeworld_probe_timer_all_gprs() != 0 }
}

#[cfg(any(
    feature = "m35c2f-ci-trap-frame-test",
    feature = "m35c2g-ci-resume-interrupt-test",
))]
fn probe_after_hlt_address() -> u64 {
    unsafe { core::ptr::addr_of!(__freeworld_probe_after_hlt) as u64 }
}

#[cfg(any(
    feature = "m35c2f-ci-trap-frame-test",
    feature = "m35c2g-ci-resume-interrupt-test",
))]
fn probe_frame_fields_match(frame: &TimerInterruptFrame) -> bool {
    let kernel_cs = u64::from(CS::get_reg().0);

    frame.rax == PROBE_RAX
        && frame.rbx == PROBE_RBX
        && frame.rcx == PROBE_RCX
        && frame.rdx == PROBE_RDX
        && frame.rsi == PROBE_RSI
        && frame.rdi == PROBE_RDI
        && frame.rbp == PROBE_RBP
        && frame.r8 == PROBE_R8
        && frame.r9 == PROBE_R9
        && frame.r10 == PROBE_R10
        && frame.r11 == PROBE_R11
        && frame.r12 == PROBE_R12
        && frame.r13 == PROBE_R13
        && frame.r14 == PROBE_R14
        && frame.r15 == PROBE_R15
        && frame.cs == kernel_cs
        && frame.rip == probe_after_hlt_address()
}

#[unsafe(no_mangle)]
extern "C" fn __freeworld_apic_timer_dispatch(frame: *mut TimerInterruptFrame) {
    let scope = interrupt_context::enter();

    #[cfg(any(
        feature = "m35c2f-ci-trap-frame-test",
        feature = "m35c2g-ci-resume-interrupt-test",
    ))]
    let probe_match = {
        let frame_address = frame as u64;

        // SAFETY: The assembly entry passes RSP after all 15 GPR pushes. Intel
        // 64 has already pushed SS, RSP, RFLAGS, CS and RIP.
        let frame_ref = unsafe { &*frame };
        if frame_ref.rip != probe_after_hlt_address() {
            false
        } else {
            let aligned = frame_address & 0xf == 0;
            let interrupted_rsp_delta = frame_ref.rsp.checked_sub(frame_address);
            let interrupted_rsp_delta_ok =
                matches!(interrupted_rsp_delta, Some(160 | 168));
            let frame_fields_ok = probe_frame_fields_match(frame_ref);

            #[cfg(feature = "m35c2f-ci-trap-frame-test")]
            crate::rt::scheduler::timer_interrupt_frame_enter(
                frame_address,
                TIMER_INTERRUPT_FRAME_BYTES as u64,
                frame_ref.rsp,
                frame_ref.rflags,
                aligned,
                interrupted_rsp_delta_ok,
                frame_fields_ok,
            );

            #[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
            crate::rt::scheduler::timer_interrupt_frame_capture_for_resume(
                frame_address,
                TIMER_INTERRUPT_FRAME_BYTES as u64,
                frame_ref.rsp,
                frame_ref.rflags,
                aligned,
                interrupted_rsp_delta_ok,
                frame_fields_ok,
            );

            true
        }
    };

    #[cfg(not(any(
        feature = "m35c2f-ci-trap-frame-test",
        feature = "m35c2g-ci-resume-interrupt-test",
        feature = "m35c2i-ci-preempt-test",
        feature = "m35c2j-ci-run-queue-test",
    )))]
    let _ = frame;

    #[cfg(feature = "m35c2j-ci-run-queue-test")]
    let run_queue_preempt_ready = {
        let frame_address = frame as u64;
        // SAFETY: The timer entry's 160-byte frame is live until this
        // dispatcher returns or transfers to another scheduler-owned stack.
        let frame_ref = unsafe { &*frame };
        crate::rt::scheduler::timer_run_queue_capture(
            frame_address,
            TIMER_INTERRUPT_FRAME_BYTES as u64,
            frame_ref.rsp,
            frame_ref.rflags,
            frame_address & 0xf == 0,
        )
    };

    #[cfg(feature = "m35c2i-ci-preempt-test")]
    let preempt_ready = {
        let frame_address = frame as u64;
        // SAFETY: The timer entry's 160-byte frame is live until this dispatcher
        // either returns to the shared IRETQ tail or hands execution away.
        let frame_ref = unsafe { &*frame };
        crate::rt::scheduler::timer_preemption_capture(
            frame_address,
            TIMER_INTERRUPT_FRAME_BYTES as u64,
            frame_ref.rsp,
            frame_ref.rflags,
            frame_address & 0xf == 0,
        )
    };

    // Tick accounting and LAPIC EOI happen before any possible handoff.
    apic::timer_interrupt();

    #[cfg(feature = "m35c2j-ci-run-queue-test")]
    if run_queue_preempt_ready {
        // EOI is complete. Drop CPU interrupt depth before rotating the real
        // run queue and restoring the selected task.
        drop(scope);
        crate::rt::scheduler::timer_run_queue_handoff();
    }

    #[cfg(feature = "m35c2i-ci-preempt-test")]
    if preempt_ready {
        // The timer frame is task-owned. EOI is complete. Drop CPU interrupt
        // depth before the timer policy hands execution to another task.
        drop(scope);
        crate::rt::scheduler::timer_preemption_handoff();
    }

    #[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
    if probe_match && crate::rt::scheduler::consume_timer_interrupt_resume_handoff() {
        // Interrupt depth is CPU handler-execution state, not task state.
        // Drop it after EOI and before abandoning A's interrupt-handler stack.
        drop(scope);
        crate::rt::scheduler::timer_interrupt_fixed_handoff_to_b();
    }

    #[cfg(feature = "m35c2f-ci-trap-frame-test")]
    if probe_match {
        crate::rt::scheduler::timer_interrupt_frame_return_same_task();
    }

    // Same-task returns leave interrupt depth before assembly restores GPRs
    // and executes IRETQ.
    drop(scope);
}

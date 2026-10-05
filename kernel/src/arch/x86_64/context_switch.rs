use core::arch::global_asm;

global_asm!(
    r#"
    .intel_syntax noprefix
    .section .text

    .global __freeworld_switch_task_context
    .type __freeworld_switch_task_context,@function
__freeworld_switch_task_context:
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15

    mov qword ptr [rdi], rsp
    mov rsp, rsi

    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx
    ret

    .global __freeworld_start_first_task
    .type __freeworld_start_first_task,@function
__freeworld_start_first_task:
    mov rsp, rdi

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
    fn __freeworld_switch_task_context(old_rsp: *mut u64, new_rsp: u64);
    fn __freeworld_start_first_task(new_rsp: u64) -> !;
}

/// Saves the current SysV callee-saved register frame on the current stack,
/// stores its RSP through old_rsp, then restores the frame at new_rsp.
///
/// # Safety
///
/// old_rsp must be writable scheduler-owned storage for the current task.
/// new_rsp must point at a valid FreeWorld SavedRegisterFrame followed by a
/// return RIP on a live task stack. The caller must hold strong references to
/// both task objects for the entire switch.
pub unsafe fn switch_task_context(old_rsp: *mut u64, new_rsp: u64) {
    unsafe { __freeworld_switch_task_context(old_rsp, new_rsp) };
}

/// Starts the first task without retaining the bootstrap stack.
///
/// # Safety
///
/// new_rsp must point at a valid initial SavedRegisterFrame on a live,
/// scheduler-owned task stack. This function never returns to the bootstrap
/// stack.
pub unsafe fn start_first_task(new_rsp: u64) -> ! {
    unsafe { __freeworld_start_first_task(new_rsp) }
}

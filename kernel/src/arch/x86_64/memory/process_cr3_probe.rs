// Test-only, single-CPU CR3 proof. There is no general CR3 scheduler API
// here: only one validated process leaf is accessed, while IF is disabled.
use core::arch::global_asm;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Cr3RoundtripRegisters {
    pub process_cr3_observed: u64,
    pub virtual_readback: u64,
    pub kernel_cr3_restored: u64,
}

global_asm!(
    r#"
    .intel_syntax noprefix
    .section .text
    .global __freeworld_m5f_cr3_roundtrip
    .type __freeworld_m5f_cr3_roundtrip,@function
__freeworld_m5f_cr3_roundtrip:
    push rbx
    mov rbx, cr3
    mov cr3, rdi
    mov r8, cr3
    mov qword ptr [rsi], rdx
    mov r9, qword ptr [rsi]
    mov cr3, rbx
    mov rax, cr3
    mov qword ptr [rcx], r8
    mov qword ptr [rcx + 8], r9
    mov qword ptr [rcx + 16], rax
    pop rbx
    ret
    "#
);

unsafe extern "C" {
    fn __freeworld_m5f_cr3_roundtrip(
        process_cr3: u64,
        user_virtual_address: u64,
        pattern: u64,
        result: *mut Cr3RoundtripRegisters,
    );
}

/// # Safety
///
/// The caller must hold the process root and mapped user leaf alive and must
/// have validated a U+RW leaf reachable from the process PML4. IF must be
/// disabled, the CPU must not be executing on that process root, and all
/// kernel text/data/stack/direct-map pages used by this routine must exist in
/// the process root's higher-half kernel mappings. The assembly does not
/// call Rust or access the memory manager while the foreign root is active.
///
/// The original full CR3 value is saved on the shared kernel stack and
/// restored before the assembly returns. There is no user-mode execution.
pub unsafe fn execute(
    process_root_phys: u64,
    user_virtual_address: u64,
    pattern: u64,
) -> Cr3RoundtripRegisters {
    let mut result = Cr3RoundtripRegisters::default();
    // SAFETY: caller established the invariants in the contract above.
    unsafe {
        __freeworld_m5f_cr3_roundtrip(
            process_root_phys,
            user_virtual_address,
            pattern,
            &mut result,
        );
    }
    result
}

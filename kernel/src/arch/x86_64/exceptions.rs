use lazy_static::lazy_static;
use x86_64::{
    registers::control::Cr2,
    structures::idt::{
        InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode,
    },
};

use super::{
    apic, interrupt_context, pic,
    gdt::{
        DOUBLE_FAULT_IST_INDEX, MACHINE_CHECK_IST_INDEX, NMI_IST_INDEX,
    },
    halt_loop, serial,
};

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();

        idt.divide_error.set_handler_fn(divide_error);
        idt.debug.set_handler_fn(debug);
        idt.breakpoint.set_handler_fn(breakpoint);
        idt.overflow.set_handler_fn(overflow);
        idt.bound_range_exceeded.set_handler_fn(bound_range_exceeded);
        idt.invalid_opcode.set_handler_fn(invalid_opcode);
        idt.device_not_available.set_handler_fn(device_not_available);

        // SAFETY: The first three IST entries are initialized in the
        // FreeWorld TSS and point at dedicated static exception stacks.
        unsafe {
            idt.non_maskable_interrupt
                .set_handler_fn(non_maskable_interrupt)
                .set_stack_index(NMI_IST_INDEX);
            idt.double_fault
                .set_handler_fn(double_fault)
                .set_stack_index(DOUBLE_FAULT_IST_INDEX);
            idt.machine_check
                .set_handler_fn(machine_check)
                .set_stack_index(MACHINE_CHECK_IST_INDEX);
        }

        idt.invalid_tss.set_handler_fn(invalid_tss);
        idt.segment_not_present.set_handler_fn(segment_not_present);
        idt.stack_segment_fault.set_handler_fn(stack_segment_fault);
        idt.general_protection_fault
            .set_handler_fn(general_protection_fault);
        idt.page_fault.set_handler_fn(page_fault);
        idt.x87_floating_point.set_handler_fn(x87_floating_point);
        idt.alignment_check.set_handler_fn(alignment_check);
        idt.simd_floating_point.set_handler_fn(simd_floating_point);
        idt.virtualization.set_handler_fn(virtualization);
        idt.cp_protection_exception
            .set_handler_fn(cp_protection_exception);
        idt.hv_injection_exception
            .set_handler_fn(hv_injection_exception);
        idt.vmm_communication_exception
            .set_handler_fn(vmm_communication_exception);
        idt.security_exception.set_handler_fn(security_exception);

        for vector in pic::MASTER_VECTOR_OFFSET..=(pic::SLAVE_VECTOR_OFFSET + 7) {
            idt[vector].set_handler_fn(pic_spurious_interrupt);
        }

        idt[apic::TIMER_VECTOR].set_handler_fn(apic_timer_interrupt);
        idt[apic::SPURIOUS_VECTOR].set_handler_fn(apic_spurious_interrupt);

        idt
    };
}

pub fn init() {
    IDT.load();
}

pub fn trigger_test_breakpoint() {
    x86_64::instructions::interrupts::int3();
}

fn fatal_marker() {
    serial::println("FreeWorldOS: FATAL");
}

fn log_frame(label: &str, stack_frame: InterruptStackFrame) {
    serial::write_fmt(format_args!(
        "FreeWorldOS: EXCEPTION: {label}\n{stack_frame:#?}\n"
    ));
}

fn fatal_no_error(label: &str, stack_frame: InterruptStackFrame) -> ! {
    let _scope = interrupt_context::enter();
    fatal_marker();
    log_frame(label, stack_frame);
    halt_loop()
}

fn fatal_with_error(label: &str, stack_frame: InterruptStackFrame, error_code: u64) -> ! {
    let _scope = interrupt_context::enter();
    fatal_marker();
    serial::write_fmt(format_args!(
        "FreeWorldOS: EXCEPTION: {label} error={error_code:#x}\n{stack_frame:#?}\n"
    ));
    halt_loop()
}

macro_rules! no_error_handler {
    ($name:ident, $label:literal) => {
        extern "x86-interrupt" fn $name(stack_frame: InterruptStackFrame) {
            fatal_no_error($label, stack_frame)
        }
    };
}

macro_rules! error_handler {
    ($name:ident, $label:literal) => {
        extern "x86-interrupt" fn $name(
            stack_frame: InterruptStackFrame,
            error_code: u64,
        ) {
            fatal_with_error($label, stack_frame, error_code)
        }
    };
}

no_error_handler!(divide_error, "#DE divide error");
no_error_handler!(debug, "#DB debug");
no_error_handler!(overflow, "#OF overflow");
no_error_handler!(bound_range_exceeded, "#BR bound range exceeded");
no_error_handler!(invalid_opcode, "#UD invalid opcode");
no_error_handler!(device_not_available, "#NM device not available");
no_error_handler!(x87_floating_point, "#MF x87 floating point");
no_error_handler!(simd_floating_point, "#XM SIMD floating point");
no_error_handler!(virtualization, "#VE virtualization");
no_error_handler!(hv_injection_exception, "#HV hypervisor injection");

error_handler!(invalid_tss, "#TS invalid TSS");
error_handler!(segment_not_present, "#NP segment not present");
error_handler!(stack_segment_fault, "#SS stack segment fault");
error_handler!(general_protection_fault, "#GP general protection");
error_handler!(alignment_check, "#AC alignment check");
error_handler!(cp_protection_exception, "#CP control protection");
error_handler!(vmm_communication_exception, "#VC VMM communication");
error_handler!(security_exception, "#SX security");

extern "x86-interrupt" fn breakpoint(stack_frame: InterruptStackFrame) {
    let _scope = interrupt_context::enter();
    serial::println("FreeWorldOS: EXCEPTION: #BP breakpoint");
    serial::write_fmt(format_args!("{stack_frame:#?}\n"));
}

/// NMI invariant: this handler must remain tiny and non-faulting.
///
/// Do not allocate, acquire locks, map pages, format complex data, or call code
/// that can fault. A fault that returns can unblock NMIs before this handler has
/// returned, allowing a nested NMI to reuse and overwrite the same IST stack.
extern "x86-interrupt" fn non_maskable_interrupt(_stack_frame: InterruptStackFrame) {
    let _scope = interrupt_context::enter();
    serial::println("FreeWorldOS: EXCEPTION: NMI");
}

extern "x86-interrupt" fn double_fault(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    let _scope = interrupt_context::enter();
    fatal_marker();
    serial::write_fmt(format_args!(
        "FreeWorldOS: EXCEPTION: #DF double fault error={error_code:#x}\n{stack_frame:#?}\n"
    ));
    halt_loop()
}

extern "x86-interrupt" fn machine_check(stack_frame: InterruptStackFrame) -> ! {
    let _scope = interrupt_context::enter();
    fatal_marker();
    serial::write_fmt(format_args!(
        "FreeWorldOS: EXCEPTION: #MC machine check\n{stack_frame:#?}\n"
    ));
    halt_loop()
}

extern "x86-interrupt" fn page_fault(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let _scope = interrupt_context::enter();
    fatal_marker();
    match Cr2::read() {
        Ok(address) => serial::write_fmt(format_args!(
            "FreeWorldOS: EXCEPTION: #PF page fault address={address:?} error={error_code:?}\n{stack_frame:#?}\n"
        )),
        Err(error) => serial::write_fmt(format_args!(
            "FreeWorldOS: EXCEPTION: #PF page fault address=<noncanonical {error:?}> error={error_code:?}\n{stack_frame:#?}\n"
        )),
    }

    halt_loop()
}

extern "x86-interrupt" fn pic_spurious_interrupt(_stack_frame: InterruptStackFrame) {
    let _scope = interrupt_context::enter();
    serial::println("FreeWorldOS: IRQ: masked legacy PIC vector");
}

extern "x86-interrupt" fn apic_timer_interrupt(_stack_frame: InterruptStackFrame) {
    let _scope = interrupt_context::enter();
    // Interrupt-context invariant: no allocation, locks, serial formatting,
    // page mapping, or scheduler work. M2 proves only atomic tick delivery.
    apic::timer_interrupt();
}

extern "x86-interrupt" fn apic_spurious_interrupt(_stack_frame: InterruptStackFrame) {
    let _scope = interrupt_context::enter();
    // xAPIC spurious interrupts do not require EOI.
}

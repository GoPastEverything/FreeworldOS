use lazy_static::lazy_static;
use x86_64::{
    registers::control::Cr2,
    structures::idt::{
        InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode,
    },
};

use crate::debug::{events, panic as panic_dump};

use super::{
    apic, interrupt_context, pic,
    gdt::{
        DOUBLE_FAULT_IST_INDEX, MACHINE_CHECK_IST_INDEX, NMI_IST_INDEX,
    },
    serial,
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

fn fatal_no_error(label: &'static str, stack_frame: InterruptStackFrame) -> ! {
    let _scope = interrupt_context::enter();
    panic_dump::fatal_exception(
        events::FATAL_EXCEPTION,
        label,
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64(),
        0,
        0,
    )
}

fn fatal_with_error(
    label: &'static str,
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    let _scope = interrupt_context::enter();
    panic_dump::fatal_exception(
        events::FATAL_EXCEPTION,
        label,
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64(),
        error_code,
        0,
    )
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
    events::record(
        events::BREAKPOINT,
        events::SUBSYSTEM_EXCEPTION,
        events::LEVEL_INFO,
        [
            stack_frame.instruction_pointer.as_u64(),
            stack_frame.stack_pointer.as_u64(),
            0,
            0,
        ],
    );
    serial::println("FreeWorldOS: EXCEPTION: #BP breakpoint");
    serial::write_fmt(format_args!("{stack_frame:#?}\n"));
}

/// NMI invariant: this handler must remain tiny and non-faulting.
///
/// Do not allocate, acquire locks, map pages, format complex data, or call code
/// that can fault. A fault that returns can unblock NMIs before this handler has
/// returned, allowing a nested NMI to reuse and overwrite the same IST stack.
extern "x86-interrupt" fn non_maskable_interrupt(stack_frame: InterruptStackFrame) {
    let _scope = interrupt_context::enter();
    events::record(
        events::NMI_RECEIVED,
        events::SUBSYSTEM_EXCEPTION,
        events::LEVEL_WARN,
        [
            stack_frame.instruction_pointer.as_u64(),
            stack_frame.stack_pointer.as_u64(),
            0,
            0,
        ],
    );
    serial::write_raw(b"FreeWorldOS: EXCEPTION: NMI\r\n");
}

extern "x86-interrupt" fn double_fault(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    let _scope = interrupt_context::enter();
    panic_dump::fatal_exception(
        events::FATAL_EXCEPTION,
        "#DF double fault",
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64(),
        error_code,
        0,
    )
}

extern "x86-interrupt" fn machine_check(stack_frame: InterruptStackFrame) -> ! {
    let _scope = interrupt_context::enter();
    panic_dump::fatal_exception(
        events::FATAL_EXCEPTION,
        "#MC machine check",
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64(),
        0,
        0,
    )
}

extern "x86-interrupt" fn page_fault(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let _scope = interrupt_context::enter();
    let fault_address = Cr2::read()
        .map(|address| address.as_u64())
        .unwrap_or(0);

    panic_dump::fatal_exception(
        events::PAGE_FAULT,
        "#PF page fault",
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64(),
        error_code.bits(),
        fault_address,
    )
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

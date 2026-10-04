use core::ptr;
use lazy_static::lazy_static;
use x86_64::{
    instructions::{
        segmentation::{Segment, CS, DS, ES, SS},
        tables::load_tss,
    },
    structures::{
        gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector},
        tss::TaskStateSegment,
    },
    VirtAddr,
};

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;
pub const NMI_IST_INDEX: u16 = 1;
pub const MACHINE_CHECK_IST_INDEX: u16 = 2;

const EXCEPTION_STACK_SIZE: usize = 5 * 4096;

#[repr(align(16))]
struct AlignedStack([u8; EXCEPTION_STACK_SIZE]);

static mut DOUBLE_FAULT_STACK: AlignedStack = AlignedStack([0; EXCEPTION_STACK_SIZE]);
static mut NMI_STACK: AlignedStack = AlignedStack([0; EXCEPTION_STACK_SIZE]);
static mut MACHINE_CHECK_STACK: AlignedStack = AlignedStack([0; EXCEPTION_STACK_SIZE]);

struct Selectors {
    code: SegmentSelector,
    data: SegmentSelector,
    tss: SegmentSelector,
}

fn stack_top(stack: *const AlignedStack) -> VirtAddr {
    let start = VirtAddr::from_ptr(stack);
    start + EXCEPTION_STACK_SIZE as u64
}

lazy_static! {
    static ref TSS: TaskStateSegment = {
        let mut tss = TaskStateSegment::new();

        // SAFETY: Each stack has static storage duration and is reserved
        // exclusively for its CPU exception IST entry.
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] =
            stack_top(ptr::addr_of!(DOUBLE_FAULT_STACK));
        tss.interrupt_stack_table[NMI_IST_INDEX as usize] =
            stack_top(ptr::addr_of!(NMI_STACK));
        tss.interrupt_stack_table[MACHINE_CHECK_IST_INDEX as usize] =
            stack_top(ptr::addr_of!(MACHINE_CHECK_STACK));

        tss
    };

    static ref GDT: (GlobalDescriptorTable, Selectors) = {
        let mut gdt = GlobalDescriptorTable::new();
        let code = gdt.append(Descriptor::kernel_code_segment());
        let data = gdt.append(Descriptor::kernel_data_segment());
        let tss = gdt.append(Descriptor::tss_segment(&TSS));

        (gdt, Selectors { code, data, tss })
    };
}

pub fn init() {
    GDT.0.load();

    // SAFETY: Selectors refer to descriptors in the static GDT loaded immediately
    // above. No userspace exists yet and interrupts remain disabled during setup.
    unsafe {
        CS::set_reg(GDT.1.code);
        DS::set_reg(GDT.1.data);
        ES::set_reg(GDT.1.data);
        SS::set_reg(GDT.1.data);
        load_tss(GDT.1.tss);
    }
}

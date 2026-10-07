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
    PrivilegeLevel, VirtAddr,
};

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;
pub const NMI_IST_INDEX: u16 = 1;
pub const MACHINE_CHECK_IST_INDEX: u16 = 2;

const EXCEPTION_STACK_SIZE: usize = 5 * 4096;
pub const RING0_PRIVILEGE_STACK_SIZE: usize = 4 * 4096;
const TSS_RING0_PRIVILEGE_LEVEL_INDEX: usize = 0;

#[repr(align(16))]
struct AlignedStack<const SIZE: usize>([u8; SIZE]);

static mut DOUBLE_FAULT_STACK: AlignedStack<EXCEPTION_STACK_SIZE> =
    AlignedStack([0; EXCEPTION_STACK_SIZE]);
static mut NMI_STACK: AlignedStack<EXCEPTION_STACK_SIZE> =
    AlignedStack([0; EXCEPTION_STACK_SIZE]);
static mut MACHINE_CHECK_STACK: AlignedStack<EXCEPTION_STACK_SIZE> =
    AlignedStack([0; EXCEPTION_STACK_SIZE]);

// M5-B provides a bootstrap CPU ring-0 privilege-transition stack. It is
// separate from the three dedicated IST stacks. No ring-3 entry exists yet.
//
// Later user-task scheduling must either update RSP0 to the current task's
// kernel stack before entering ring 3, or deliberately retain a per-CPU entry
// stack and transfer the frame before a task can be switched away. M5-B makes
// no scheduling claim for user tasks.
static mut RING0_PRIVILEGE_STACK: AlignedStack<RING0_PRIVILEGE_STACK_SIZE> =
    AlignedStack([0; RING0_PRIVILEGE_STACK_SIZE]);

#[derive(Clone, Copy)]
struct Selectors {
    code: SegmentSelector,
    data: SegmentSelector,
    user_data: SegmentSelector,
    user_code: SegmentSelector,
    tss: SegmentSelector,
}

fn stack_top<const SIZE: usize>(stack: *const AlignedStack<SIZE>) -> VirtAddr {
    let start = VirtAddr::from_ptr(stack);
    start + SIZE as u64
}

fn ring0_privilege_stack_bottom() -> VirtAddr {
    VirtAddr::from_ptr(ptr::addr_of!(RING0_PRIVILEGE_STACK))
}

pub fn ring0_privilege_stack_top() -> VirtAddr {
    stack_top(ptr::addr_of!(RING0_PRIVILEGE_STACK))
}

lazy_static! {
    static ref TSS: TaskStateSegment = {
        let mut tss = TaskStateSegment::new();

        // A CPU transition from CPL3 to CPL0 uses privilege_stack_table[0].
        // M5-B installs only the stack; no outward transition is performed.
        tss.privilege_stack_table[TSS_RING0_PRIVILEGE_LEVEL_INDEX] =
            ring0_privilege_stack_top();

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

        // Keep the selector order deliberate:
        //   kernel code, kernel data, user data, user code, TSS.
        // The user-data/user-code adjacency is compatible with the fixed
        // SYSRET selector offsets, but M5-B does not program STAR/LSTAR.
        let code = gdt.append(Descriptor::kernel_code_segment());
        let data = gdt.append(Descriptor::kernel_data_segment());

        let user_data_gdt = gdt.append(Descriptor::user_data_segment());
        let user_code_gdt = gdt.append(Descriptor::user_code_segment());

        let user_data =
            SegmentSelector::new(user_data_gdt.index(), PrivilegeLevel::Ring3);
        let user_code =
            SegmentSelector::new(user_code_gdt.index(), PrivilegeLevel::Ring3);

        let tss = gdt.append(Descriptor::tss_segment(&TSS));

        assert_eq!(
            data.index(),
            code.index() + 1,
            "FreeWorld kernel code/data GDT adjacency changed"
        );
        assert_eq!(
            user_data.index(),
            data.index() + 1,
            "FreeWorld user data selector no longer follows kernel data"
        );
        assert_eq!(
            user_code.index(),
            user_data.index() + 1,
            "FreeWorld user code/data SYSRET-compatible order changed"
        );

        (
            gdt,
            Selectors {
                code,
                data,
                user_data,
                user_code,
                tss,
            },
        )
    };
}

pub fn init() {
    GDT.0.load();

    // SAFETY: Selectors refer to descriptors in the static GDT loaded
    // immediately above. Ring 3 is not entered by M5-B.
    unsafe {
        CS::set_reg(GDT.1.code);
        DS::set_reg(GDT.1.data);
        ES::set_reg(GDT.1.data);
        SS::set_reg(GDT.1.data);
        load_tss(GDT.1.tss);
    }
}

pub fn user_code_selector() -> SegmentSelector {
    GDT.1.user_code
}

pub fn user_data_selector() -> SegmentSelector {
    GDT.1.user_data
}

fn selector_bits(selector: SegmentSelector, privilege: PrivilegeLevel) -> u16 {
    (selector.index() << 3) | privilege as u16
}

#[cfg(feature = "m5b-ci-self-test")]
pub fn ci_self_test() {
    const BOTTOM_PATTERN: u64 = 0x4d35_4252_5350_3042;
    const TOP_PATTERN: u64 = 0x4d35_4252_5350_3054;

    let user_data = user_data_selector();
    let user_code = user_code_selector();

    assert_eq!(
        user_data,
        SegmentSelector::new(user_data.index(), PrivilegeLevel::Ring3)
    );
    assert_eq!(
        user_code,
        SegmentSelector::new(user_code.index(), PrivilegeLevel::Ring3)
    );
    assert_eq!(user_code.index(), user_data.index() + 1);

    assert_eq!(
        CS::get_reg(),
        GDT.1.code,
        "FreeWorld M5-B changed current kernel CS"
    );
    assert_eq!(
        SS::get_reg(),
        GDT.1.data,
        "FreeWorld M5-B changed current kernel SS"
    );

    let bottom = ring0_privilege_stack_bottom().as_u64();
    let top = ring0_privilege_stack_top().as_u64();

    assert_eq!(top - bottom, RING0_PRIVILEGE_STACK_SIZE as u64);
    assert_eq!(top & 0xf, 0, "FreeWorld TSS RSP0 is not 16-byte aligned");
    assert!(
        crate::memory::address_space::is_kernel_address(bottom)
            && crate::memory::address_space::is_kernel_address(top - 1),
        "FreeWorld TSS RSP0 stack escaped the kernel half"
    );
    assert_eq!(
        TSS.privilege_stack_table[TSS_RING0_PRIVILEGE_LEVEL_INDEX],
        ring0_privilege_stack_top(),
        "FreeWorld TSS RSP0 does not reference the privilege stack"
    );

    // SAFETY: These volatile accesses stay inside the statically reserved
    // M5-B privilege stack and do not create lasting references to static mut.
    unsafe {
        core::ptr::write_volatile(bottom as *mut u64, BOTTOM_PATTERN);
        core::ptr::write_volatile((top - 8) as *mut u64, TOP_PATTERN);

        assert_eq!(
            core::ptr::read_volatile(bottom as *const u64),
            BOTTOM_PATTERN
        );
        assert_eq!(
            core::ptr::read_volatile((top - 8) as *const u64),
            TOP_PATTERN
        );
    }

    let user_ss_bits = selector_bits(user_data, PrivilegeLevel::Ring3);
    let user_cs_bits = selector_bits(user_code, PrivilegeLevel::Ring3);

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M5-B privilege setup: user_ss={user_ss_bits:#x} user_cs={user_cs_bits:#x} rsp0={top:#x} stack_bytes={} gdt_order=kernel_code>kernel_data>user_data>user_code>tss\n",
        RING0_PRIVILEGE_STACK_SIZE,
    ));
    crate::arch::serial::println(
        "FreeWorldOS: M5-B privilege self-test: passed user_code=dpl3 user_data=dpl3 rpl3=ok sysret_order=ok tss_rsp0=higher stack_writable=ok current_cpl=ring0 ring3_entry=off callgate=off",
    );
}

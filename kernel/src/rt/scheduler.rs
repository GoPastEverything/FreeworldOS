use alloc::sync::Arc;
use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};

use crate::{
    arch,
    object::{
        self,
        handle::Rights,
        task::{SavedContextKind, TaskObject, TaskState},
        FwObject, ObjectRef,
    },
};

const CURRENT_NONE: u8 = 0;
const CURRENT_A: u8 = 1;
const CURRENT_B: u8 = 2;

struct SchedulerState {
    task_a: ObjectRef,
    task_b: ObjectRef,
    current: AtomicU8,
}

impl SchedulerState {
    fn task_a(&self) -> &TaskObject {
        object::task_from_ref(&self.task_a)
            .expect("FreeWorld scheduler task A reference changed type")
    }

    fn task_b(&self) -> &TaskObject {
        object::task_from_ref(&self.task_b)
            .expect("FreeWorld scheduler task B reference changed type")
    }
}

struct SchedulerStorage(UnsafeCell<MaybeUninit<SchedulerState>>);

// SAFETY: C2c is bootstrap-CPU-only. The scheduler object references are
// written once before publication and never replaced. Runtime mutation is
// limited to atomics inside SchedulerState/TaskObject. SMP scheduling is not
// enabled by this milestone.
unsafe impl Sync for SchedulerStorage {}

static SCHEDULER: SchedulerStorage =
    SchedulerStorage(UnsafeCell::new(MaybeUninit::uninit()));
static SCHEDULER_INITIALIZED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "m35c2f-ci-trap-frame-test")]
static C2F_FRAME_OBSERVED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2f-ci-trap-frame-test")]
static C2F_FRAME_KIND_OK: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2f-ci-trap-frame-test")]
static C2F_FRAME_IN_STACK: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2f-ci-trap-frame-test")]
static C2F_HARDWARE_RSP_IN_STACK: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2f-ci-trap-frame-test")]
static C2F_ENTRY_ALIGNMENT_OK: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2f-ci-trap-frame-test")]
static C2F_INTERRUPTED_IF_ON: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2f-ci-trap-frame-test")]
static C2F_SAME_TASK_RETURN_READY: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2f-ci-trap-frame-test")]
static C2F_FRAME_ADDRESS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2f-ci-trap-frame-test")]
static C2F_FRAME_FIELDS_OK: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2f-ci-trap-frame-test")]
static C2F_RSP_DELTA_OK: AtomicBool = AtomicBool::new(false);


fn install(task_a: ObjectRef, task_b: ObjectRef) {
    assert!(
        !SCHEDULER_INITIALIZED.load(Ordering::Acquire),
        "FreeWorld C2c scheduler installed twice"
    );

    // SAFETY: Single bootstrap CPU, one initialization before publication.
    unsafe {
        (*SCHEDULER.0.get()).write(SchedulerState {
            task_a,
            task_b,
            current: AtomicU8::new(CURRENT_NONE),
        });
    }

    SCHEDULER_INITIALIZED.store(true, Ordering::Release);
}

fn scheduler() -> &'static SchedulerState {
    assert!(
        SCHEDULER_INITIALIZED.load(Ordering::Acquire),
        "FreeWorld scheduler used before initialization"
    );

    // SAFETY: Acquire observed one-time publication. C2c never removes or
    // replaces this state.
    unsafe { &*(*SCHEDULER.0.get()).as_ptr() }
}

#[cfg(any(
    feature = "m35c2c-ci-switch-test",
    feature = "m35c2d-ci-roundtrip-test",
    feature = "m35c2e-ci-irq-stack-test",
))]
pub fn ci_voluntary_switch_test() -> ! {
    let task_a_ref = object::create_task_ref()
        .expect("C2c failed to create scheduler-owned task A");
    let task_b_ref = object::create_task_ref()
        .expect("C2c failed to create scheduler-owned task B");

    // Give each task a temporary handle, then close it while the task is still
    // Created. The Arc moved into the scheduler is therefore the lifetime
    // owner once execution begins; runnable stack lifetime does not depend on
    // handle-table behavior.
    let task_a_handle = object::install_handle_for_ref(&task_a_ref, Rights::READ)
        .expect("C2c failed to install task A handle");
    let task_b_handle = object::install_handle_for_ref(&task_b_ref, Rights::READ)
        .expect("C2c failed to install task B handle");

    object::close(task_a_handle).expect("C2c failed to close task A bootstrap handle");
    object::close(task_b_handle).expect("C2c failed to close task B bootstrap handle");

    object::task_from_ref(&task_a_ref)
        .expect("C2c task A reference changed type")
        .prepare_initial_context(task_a_entry)
        .expect("C2c failed to prepare task A saved frame");
    object::task_from_ref(&task_b_ref)
        .expect("C2c task B reference changed type")
        .prepare_initial_context(task_b_entry)
        .expect("C2c failed to prepare task B saved frame");

    install(task_a_ref, task_b_ref);

    let state = scheduler();
    assert_eq!(
        Arc::strong_count(&state.task_a),
        1,
        "C2c task A has an unexpected owner outside the scheduler"
    );
    assert_eq!(
        Arc::strong_count(&state.task_b),
        1,
        "C2c task B has an unexpected owner outside the scheduler"
    );

    let task_a = state.task_a();
    let first_rsp = task_a.saved_stack_pointer();
    assert!(
        task_a.saved_stack_pointer_in_stack(),
        "C2c task A initial saved RSP is outside its stack"
    );
    assert!(
        arch::interrupts_enabled(),
        "C2c first task start requires IF enabled before scheduler handoff"
    );

    crate::arch::serial::println(
        "FreeWorldOS: M3.5-C2c scheduler refs: owned handles=closed",
    );

    // No maskable interrupt may observe Running/current state before the CPU is
    // actually executing A's stack. NMIs do not inspect scheduler state.
    arch::disable_interrupts();
    task_a
        .begin_running_from_saved()
        .expect("C2c task A could not transition Runnable -> Running");
    state.current.store(CURRENT_A, Ordering::Release);

    // SAFETY: task A is Running, the scheduler owns its sole strong reference,
    // first_rsp points at the validated initial SavedRegisterFrame, and IF is
    // clear across the handoff. start_first_task re-enables IF on A's stack.
    unsafe { arch::start_first_task(first_rsp) }
}

#[cfg(any(
    feature = "m35c2c-ci-switch-test",
    feature = "m35c2d-ci-roundtrip-test",
    feature = "m35c2e-ci-irq-stack-test",
))]
extern "C" fn task_a_entry() -> ! {
    assert!(
        arch::interrupts_enabled(),
        "C2c task A entered with IF disabled"
    );
    let state = scheduler();
    let task_a = state.task_a();

    assert_eq!(
        state.current.load(Ordering::Acquire),
        CURRENT_A,
        "C2c entered task A while scheduler current != A"
    );
    assert_eq!(
        task_a.state(),
        TaskState::Running,
        "C2c task A did not enter Running state"
    );

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M3.5-C2c task A: running task_id={}\n",
        task_a.info().id,
    ));

    yield_a_to_b();

    #[cfg(feature = "m35c2c-ci-switch-test")]
    panic!("C2c task A resumed after the one-way A -> B proof");

    #[cfg(any(
    feature = "m35c2d-ci-roundtrip-test",
    feature = "m35c2e-ci-irq-stack-test",
))]
    {
        let state = scheduler();
        let task_a = state.task_a();
        let task_b = state.task_b();

        assert_eq!(
            state.current.load(Ordering::Acquire),
            CURRENT_A,
            "C2d resumed A while scheduler current != A"
        );
        assert_eq!(task_a.state(), TaskState::Running);
        assert_eq!(task_b.state(), TaskState::Runnable);
        assert!(task_b.saved_stack_pointer_present());
        assert!(task_b.saved_stack_pointer_in_stack());
        assert!(arch::interrupts_enabled());

        crate::arch::serial::write_fmt(format_args!(
            "FreeWorldOS: M3.5-C2d task A: resumed saved_b_rsp={:#x} if=on\n",
            task_b.saved_stack_pointer(),
        ));

        #[cfg(feature = "m35c2e-ci-irq-stack-test")]
        wait_for_timer_tick_on_task_stack("A");

        #[cfg(feature = "m35c2d-ci-roundtrip-test")]
        crate::arch::serial::println(
            "FreeWorldOS: M3.5-C2d round trip: passed A->B->A scheduler_refs=owned interrupt_window=closed if_policy=resume_enabled timer_preemption=off",
        );

        #[cfg(feature = "m35c2e-ci-irq-stack-test")]
        crate::arch::serial::println(
            "FreeWorldOS: M3.5-C2e irq-on-task-stack: passed A=ok B=ok target_rsp_under_cli=ok timer_preemption=off",
        );

        arch::halt_loop()
    }
}

#[cfg(any(
    feature = "m35c2c-ci-switch-test",
    feature = "m35c2d-ci-roundtrip-test",
    feature = "m35c2e-ci-irq-stack-test",
))]
fn yield_a_to_b() {
    let state = scheduler();
    let task_a = state.task_a();
    let task_b = state.task_b();

    assert_eq!(
        state.current.load(Ordering::Acquire),
        CURRENT_A,
        "C2c yield requested when task A was not current"
    );

    assert!(
        arch::interrupts_enabled(),
        "C2c voluntary yield requires IF enabled on entry"
    );

    // Close the stale/changed-target window before reading the incoming saved
    // RSP. This is the ordering template for later timer-selected switching.
    arch::disable_interrupts();
    let new_rsp = task_b.saved_stack_pointer();
    assert!(
        task_b.saved_stack_pointer_in_stack(),
        "C2c task B saved RSP is outside its stack"
    );

    // From this point until B is executing its restored stack, maskable
    // interrupts remain disabled.
    task_a
        .prepare_running_context_save()
        .expect("C2c task A could not prepare Running -> Runnable save");
    task_b
        .begin_running_from_saved()
        .expect("C2c task B could not transition Runnable -> Running");
    state.current.store(CURRENT_B, Ordering::Release);

    // SAFETY: The scheduler owns both task references. task A's storage is
    // writable and its stack remains live; task B has a validated saved frame.
    // IF is clear during the state/RSP handoff and the assembly path re-enables
    // it only after B's register frame is restored on B's stack.
    unsafe {
        arch::switch_task_context(
            task_a.saved_stack_pointer_storage(),
            new_rsp,
        );
    }

    assert!(
        arch::interrupts_enabled(),
        "C2d resumed task A with IF disabled"
    );
}

#[cfg(any(
    feature = "m35c2c-ci-switch-test",
    feature = "m35c2d-ci-roundtrip-test",
    feature = "m35c2e-ci-irq-stack-test",
))]
extern "C" fn task_b_entry() -> ! {
    assert!(
        arch::interrupts_enabled(),
        "C2c task B entered with IF disabled"
    );
    let state = scheduler();
    let task_a = state.task_a();
    let task_b = state.task_b();

    assert_eq!(
        state.current.load(Ordering::Acquire),
        CURRENT_B,
        "C2c entered task B while scheduler current != B"
    );
    assert_eq!(
        task_a.state(),
        TaskState::Runnable,
        "C2c task A was not Runnable after yielding"
    );
    assert_eq!(
        task_b.state(),
        TaskState::Running,
        "C2c task B did not enter Running state"
    );
    assert!(
        task_a.saved_stack_pointer_present(),
        "C2c task A did not publish saved stack state"
    );
    assert!(
        task_a.saved_stack_pointer_in_stack(),
        "C2c task A switch-saved RSP is outside its stack"
    );
    assert_eq!(
        Arc::strong_count(&state.task_a),
        1,
        "C2c scheduler lost sole lifetime ownership of task A"
    );
    assert_eq!(
        Arc::strong_count(&state.task_b),
        1,
        "C2c scheduler lost sole lifetime ownership of task B"
    );

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M3.5-C2c task B: running task_id={} saved_a_rsp={:#x}\n",
        task_b.info().id,
        task_a.saved_stack_pointer(),
    ));
    #[cfg(feature = "m35c2c-ci-switch-test")]
    {
        crate::arch::serial::println(
            "FreeWorldOS: M3.5-C2c voluntary switch: passed A->B scheduler_refs=owned timer_preemption=off",
        );
        arch::halt_loop()
    }

    #[cfg(any(
    feature = "m35c2d-ci-roundtrip-test",
    feature = "m35c2e-ci-irq-stack-test",
))]
    {
        #[cfg(feature = "m35c2e-ci-irq-stack-test")]
        wait_for_timer_tick_on_task_stack("B");

        crate::arch::serial::println(
            "FreeWorldOS: M3.5-C2d task B: yielding back to A if=on",
        );
        yield_b_to_a();
        panic!("C2d task B resumed after round-trip proof");
    }
}


#[cfg(any(
    feature = "m35c2d-ci-roundtrip-test",
    feature = "m35c2e-ci-irq-stack-test",
))]
pub fn ci_interrupt_safe_roundtrip_test() -> ! {
    ci_voluntary_switch_test()
}

#[cfg(any(
    feature = "m35c2d-ci-roundtrip-test",
    feature = "m35c2e-ci-irq-stack-test",
))]
fn yield_b_to_a() {
    let state = scheduler();
    let task_a = state.task_a();
    let task_b = state.task_b();

    assert_eq!(
        state.current.load(Ordering::Acquire),
        CURRENT_B,
        "C2d B->A yield requested when task B was not current"
    );
    assert!(
        arch::interrupts_enabled(),
        "C2d B->A voluntary yield requires IF enabled on entry"
    );

    arch::disable_interrupts();
    let new_rsp = task_a.saved_stack_pointer();
    assert!(
        task_a.saved_stack_pointer_in_stack(),
        "C2d task A saved RSP is outside its stack before resume"
    );
    task_b
        .prepare_running_context_save()
        .expect("C2d task B could not prepare Running -> Runnable save");
    task_a
        .begin_running_from_saved()
        .expect("C2d task A could not transition Runnable -> Running");
    state.current.store(CURRENT_A, Ordering::Release);

    // SAFETY: The scheduler owns both tasks, IF is clear across state/RSP
    // publication, B's old RSP storage is writable, and A's saved frame is
    // validated. The assembly path restores A and re-enables IF on A's stack.
    unsafe {
        arch::switch_task_context(
            task_b.saved_stack_pointer_storage(),
            new_rsp,
        );
    }

    assert!(
        arch::interrupts_enabled(),
        "C2d resumed task B with IF disabled"
    );
}


#[cfg(feature = "m35c2e-ci-irq-stack-test")]
pub fn ci_irq_on_task_stack_test() -> ! {
    ci_voluntary_switch_test()
}

#[cfg(feature = "m35c2e-ci-irq-stack-test")]
fn wait_for_timer_tick_on_task_stack(task_name: &str) {
    assert!(
        arch::interrupts_enabled(),
        "C2e timer IRQ proof requires IF enabled"
    );

    let before = super::time::now().0;
    loop {
        // SAFETY: IF is enabled, so HLT sleeps until an interrupt. The loop
        // only succeeds when the existing LAPIC timer handler advances the
        // architecture-neutral tick count and returns to this same task.
        unsafe {
            core::arch::asm!("hlt", options(nostack, preserves_flags));
        }

        let after = super::time::now().0;
        if after != before {
            crate::arch::serial::write_fmt(format_args!(
                "FreeWorldOS: M3.5-C2e irq_on_task_stack=ok task={} before={} after={}\n",
                task_name,
                before,
                after,
            ));
            return;
        }
    }
}


#[cfg(feature = "m35c2f-ci-trap-frame-test")]
pub fn ci_timer_trap_frame_test() -> ! {
    let task_a_ref = object::create_task_ref()
        .expect("C2f failed to create scheduler-owned task A");
    let task_b_ref = object::create_task_ref()
        .expect("C2f failed to create unused task B lifetime anchor");

    let task_a_handle = object::install_handle_for_ref(&task_a_ref, Rights::READ)
        .expect("C2f failed to install task A bootstrap handle");
    object::close(task_a_handle)
        .expect("C2f failed to close task A bootstrap handle");

    object::task_from_ref(&task_a_ref)
        .expect("C2f task A reference changed type")
        .prepare_initial_context(task_timer_trap_entry)
        .expect("C2f failed to prepare task A voluntary entry frame");

    install(task_a_ref, task_b_ref);

    let state = scheduler();
    assert_eq!(
        Arc::strong_count(&state.task_a),
        1,
        "C2f task A lifetime is not scheduler-owned"
    );

    let task_a = state.task_a();
    let first_rsp = task_a.saved_stack_pointer();
    assert!(task_a.saved_stack_pointer_in_stack());
    assert_eq!(task_a.saved_context_kind(), SavedContextKind::Voluntary);
    assert!(arch::interrupts_enabled());

    crate::arch::serial::println(
        "FreeWorldOS: M3.5-C2f timer frame: starting same-task iretq proof",
    );

    arch::disable_interrupts();
    task_a
        .begin_running_from_saved()
        .expect("C2f task A could not transition Runnable -> Running");
    state.current.store(CURRENT_A, Ordering::Release);

    // SAFETY: task A owns a valid voluntary entry frame and the scheduler owns
    // its lifetime. start_first_task enables IF only after A's stack is live.
    unsafe { arch::start_first_task(first_rsp) }
}

#[cfg(feature = "m35c2f-ci-trap-frame-test")]
extern "C" fn task_timer_trap_entry() -> ! {
    let state = scheduler();
    let task = state.task_a();

    assert_eq!(state.current.load(Ordering::Acquire), CURRENT_A);
    assert_eq!(task.state(), TaskState::Running);
    assert_eq!(task.saved_context_kind(), SavedContextKind::None);
    assert!(!task.saved_stack_pointer_present());
    assert!(arch::interrupts_enabled());

    let before = super::time::now().0;
    let mut all_gprs_ok = true;

    while super::time::now().0 == before {
        // SAFETY: IF is enabled and the periodic LAPIC timer is already live.
        // The assembly probe verifies that all fifteen GPR values survive the
        // timer entry/return before restoring the caller's callee-saved set.
        all_gprs_ok &= unsafe { arch::probe_timer_all_gprs_once() };
    }

    let after = super::time::now().0;
    assert!(after > before);
    assert!(all_gprs_ok, "C2f timer return changed a general-purpose register");
    assert!(C2F_FRAME_OBSERVED.load(Ordering::Acquire));
    assert!(C2F_FRAME_KIND_OK.load(Ordering::Acquire));
    assert!(C2F_FRAME_IN_STACK.load(Ordering::Acquire));
    assert!(C2F_HARDWARE_RSP_IN_STACK.load(Ordering::Acquire));
    assert!(C2F_ENTRY_ALIGNMENT_OK.load(Ordering::Acquire));
    assert!(C2F_INTERRUPTED_IF_ON.load(Ordering::Acquire));
    assert!(C2F_FRAME_FIELDS_OK.load(Ordering::Acquire));
    assert!(C2F_RSP_DELTA_OK.load(Ordering::Acquire));
    assert!(C2F_SAME_TASK_RETURN_READY.load(Ordering::Acquire));
    assert_eq!(task.saved_context_kind(), SavedContextKind::None);
    assert!(!task.saved_stack_pointer_present());
    assert!(!arch::in_interrupt());

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M3.5-C2f timer frame: frame={:#x} bytes=160 alignment=ok kind=interrupt in_stack=ok hardware_rsp=ok rsp_delta=160|168 frame_fields=ok interrupted_if=on all_gprs=ok depth=clear eoi_before_scheduler=locked iret_same_task=ok\n",
        C2F_FRAME_ADDRESS.load(Ordering::Acquire),
    ));
    crate::arch::serial::println(
        "FreeWorldOS: M3.5-C2f trap-frame proof: passed timer_only=ok switch=off",
    );

    arch::halt_loop()
}

#[cfg(feature = "m35c2f-ci-trap-frame-test")]
pub(crate) fn timer_interrupt_frame_enter(
    frame_rsp: u64,
    frame_bytes: u64,
    hardware_rsp: u64,
    rflags: u64,
    aligned: bool,
    rsp_delta_ok: bool,
    frame_fields_ok: bool,
) {
    if !SCHEDULER_INITIALIZED.load(Ordering::Acquire) {
        return;
    }

    let state = scheduler();
    if state.current.load(Ordering::Acquire) != CURRENT_A {
        return;
    }

    let task = state.task_a();
    let info = task.info();

    task.observe_interrupt_context(frame_rsp, frame_bytes)
        .expect("C2f failed to publish timer interrupt frame");

    let hardware_rsp_in_stack =
        hardware_rsp >= info.stack_bottom && hardware_rsp <= info.stack_top;

    C2F_FRAME_ADDRESS.store(frame_rsp, Ordering::Release);
    C2F_FRAME_OBSERVED.store(true, Ordering::Release);
    C2F_FRAME_KIND_OK.store(
        task.saved_context_kind() == SavedContextKind::Interrupt,
        Ordering::Release,
    );
    C2F_FRAME_IN_STACK.store(task.saved_stack_pointer_in_stack(), Ordering::Release);
    C2F_HARDWARE_RSP_IN_STACK.store(hardware_rsp_in_stack, Ordering::Release);
    C2F_ENTRY_ALIGNMENT_OK.store(aligned, Ordering::Release);
    C2F_INTERRUPTED_IF_ON.store(rflags & (1 << 9) != 0, Ordering::Release);
    C2F_RSP_DELTA_OK.store(rsp_delta_ok, Ordering::Release);
    C2F_FRAME_FIELDS_OK.store(frame_fields_ok, Ordering::Release);
}

#[cfg(feature = "m35c2f-ci-trap-frame-test")]
pub(crate) fn timer_interrupt_frame_return_same_task() {
    if !SCHEDULER_INITIALIZED.load(Ordering::Acquire) {
        return;
    }

    let state = scheduler();
    if state.current.load(Ordering::Acquire) != CURRENT_A {
        return;
    }

    let task = state.task_a();
    if task.saved_context_kind() != SavedContextKind::Interrupt {
        return;
    }

    assert!(task.saved_stack_pointer_in_stack());
    task.finish_same_task_interrupt_context()
        .expect("C2f failed to consume same-task interrupt frame");
    C2F_SAME_TASK_RETURN_READY.store(true, Ordering::Release);
}

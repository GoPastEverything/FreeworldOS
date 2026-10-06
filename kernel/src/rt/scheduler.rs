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
static C2F_PROBE_HITS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2f-ci-trap-frame-test")]
static C2F_FRAME_FIELDS_OK: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2f-ci-trap-frame-test")]
static C2F_RSP_DELTA_OK: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
static C2G_ARMED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
static C2G_CAPTURED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
static C2G_B_RAN: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
static C2G_RESUME_ISSUED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
static C2G_FRAME_ADDRESS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
static C2G_PROBE_HITS: AtomicU64 = AtomicU64::new(0);


#[cfg(feature = "m35c2i-ci-preempt-test")]
static C2I_SWITCHES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2i-ci-preempt-test")]
static C2I_A_RUNS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2i-ci-preempt-test")]
static C2I_B_RUNS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2i-ci-preempt-test")]
static C2I_A_TO_B: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2i-ci-preempt-test")]
static C2I_B_TO_A: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2i-ci-preempt-test")]
static C2I_VOLUNTARY_STARTS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2i-ci-preempt-test")]
static C2I_INTERRUPT_RESUMES: AtomicU64 = AtomicU64::new(0);

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

    while C2F_PROBE_HITS.load(Ordering::Acquire) == 0 {
        // SAFETY: IF is enabled and the periodic LAPIC timer is already live.
        // A non-probe interrupt may wake HLT, but it does not count. C2f loops
        // until a timer frame whose RIP is the exact post-HLT probe label has
        // been observed.
        all_gprs_ok &= unsafe { arch::probe_timer_all_gprs_once() };
    }

    let after = super::time::now().0;
    assert!(after > before);
    assert!(C2F_PROBE_HITS.load(Ordering::Acquire) >= 1);
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
    C2F_PROBE_HITS.fetch_add(1, Ordering::AcqRel);
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


#[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
pub fn ci_resume_interrupt_frame_test() -> ! {
    let task_a_ref = object::create_task_ref()
        .expect("C2g failed to create scheduler-owned task A");
    let task_b_ref = object::create_task_ref()
        .expect("C2g failed to create scheduler-owned task B");

    let task_a_handle = object::install_handle_for_ref(&task_a_ref, Rights::READ)
        .expect("C2g failed to install task A bootstrap handle");
    let task_b_handle = object::install_handle_for_ref(&task_b_ref, Rights::READ)
        .expect("C2g failed to install task B bootstrap handle");
    object::close(task_a_handle)
        .expect("C2g failed to close task A bootstrap handle");
    object::close(task_b_handle)
        .expect("C2g failed to close task B bootstrap handle");

    object::task_from_ref(&task_a_ref)
        .expect("C2g task A reference changed type")
        .prepare_initial_context(task_interrupt_resume_a_entry)
        .expect("C2g failed to prepare task A voluntary entry frame");
    object::task_from_ref(&task_b_ref)
        .expect("C2g task B reference changed type")
        .prepare_initial_context(task_interrupt_resume_b_entry)
        .expect("C2g failed to prepare task B voluntary entry frame");

    install(task_a_ref, task_b_ref);

    let state = scheduler();
    assert_eq!(Arc::strong_count(&state.task_a), 1);
    assert_eq!(Arc::strong_count(&state.task_b), 1);

    let task_a = state.task_a();
    let first_rsp = task_a.saved_stack_pointer();
    assert!(task_a.saved_stack_pointer_in_stack());
    assert_eq!(task_a.saved_context_kind(), SavedContextKind::Voluntary);
    assert!(arch::interrupts_enabled());

    crate::arch::serial::println(
        "FreeWorldOS: M3.5-C2g interrupt resume: starting capture->B->iretq(A) proof",
    );

    arch::disable_interrupts();
    task_a
        .begin_running_from_saved()
        .expect("C2g task A could not transition Runnable -> Running");
    state.current.store(CURRENT_A, Ordering::Release);

    // SAFETY: A owns a valid voluntary frame and the scheduler owns both task
    // lifetimes. start_first_task enables IF only after A's stack is active.
    unsafe { arch::start_first_task(first_rsp) }
}

#[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
extern "C" fn task_interrupt_resume_a_entry() -> ! {
    let state = scheduler();
    let task_a = state.task_a();

    assert_eq!(state.current.load(Ordering::Acquire), CURRENT_A);
    assert_eq!(task_a.state(), TaskState::Running);
    assert_eq!(task_a.saved_context_kind(), SavedContextKind::None);
    assert!(arch::interrupts_enabled());

    C2G_ARMED.store(true, Ordering::Release);

    // SAFETY: IF is enabled and the periodic LAPIC timer is running. A
    // non-target wake does not count; loop until the exact post-HLT timer frame
    // is captured, handed to B, and explicitly resumed through IRETQ.
    let mut all_gprs_ok = true;
    while C2G_PROBE_HITS.load(Ordering::Acquire) == 0 {
        all_gprs_ok &= unsafe { arch::probe_timer_all_gprs_once() };
    }

    assert!(all_gprs_ok, "C2g IRETQ resume changed a general-purpose register");
    assert!(
        !C2G_CAPTURED.load(Ordering::Acquire),
        "C2h one-shot handoff flag was not consumed"
    );
    assert!(C2G_B_RAN.load(Ordering::Acquire));
    assert!(C2G_RESUME_ISSUED.load(Ordering::Acquire));
    assert!(C2G_PROBE_HITS.load(Ordering::Acquire) >= 1);
    assert_eq!(state.current.load(Ordering::Acquire), CURRENT_A);
    assert_eq!(task_a.state(), TaskState::Running);
    assert_eq!(task_a.saved_context_kind(), SavedContextKind::None);
    assert!(!task_a.saved_stack_pointer_present());
    assert_eq!(state.task_b().state(), TaskState::Stopped);
    assert!(!arch::in_interrupt());
    assert!(arch::interrupts_enabled());

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M3.5-C2g interrupt resume: frame={:#x} kind=interrupt saved_on=A B_observed=ok eoi_before_handoff=ok depth_before_handoff=clear resume_path=iretq all_gprs=ok if_restored=ok handoff_one_shot=ok shared_iretq_tail=ok\n",
        C2G_FRAME_ADDRESS.load(Ordering::Acquire),
    ));
    crate::arch::serial::println(
        "FreeWorldOS: M3.5-C2g resume proof: passed fixed_handoff=A->B explicit_resume=B->A timer_selection=off",
    );

    arch::halt_loop()
}

#[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
extern "C" fn task_interrupt_resume_b_entry() -> ! {
    let state = scheduler();
    let task_a = state.task_a();
    let task_b = state.task_b();

    assert_eq!(state.current.load(Ordering::Acquire), CURRENT_B);
    assert_eq!(task_a.state(), TaskState::Runnable);
    assert_eq!(task_a.saved_context_kind(), SavedContextKind::Interrupt);
    assert!(task_a.saved_stack_pointer_present());
    assert!(task_a.saved_stack_pointer_in_stack());
    assert_eq!(task_b.state(), TaskState::Running);
    assert_eq!(task_b.saved_context_kind(), SavedContextKind::None);
    assert!(!arch::in_interrupt());
    assert!(arch::interrupts_enabled());

    C2G_B_RAN.store(true, Ordering::Release);
    crate::arch::serial::println(
        "FreeWorldOS: M3.5-C2g task B: observed A Interrupt frame after EOI/depth teardown",
    );

    // The target Interrupt RSP is read only after CLI. No timer-selected policy
    // exists here: B explicitly resumes the one predetermined task A.
    arch::disable_interrupts();
    let a_rsp = task_a.saved_stack_pointer();
    assert_eq!(task_a.saved_context_kind(), SavedContextKind::Interrupt);
    assert!(task_a.saved_stack_pointer_in_stack());

    task_b
        .stop_running_without_saved_context()
        .expect("C2g could not stop deterministic handoff task B");
    task_a
        .begin_running_from_interrupt()
        .expect("C2g could not transition A Interrupt frame to Running");
    state.current.store(CURRENT_A, Ordering::Release);
    C2G_RESUME_ISSUED.store(true, Ordering::Release);

    // SAFETY: IF is clear, A's 160-byte Interrupt frame remains live on A's
    // scheduler-owned stack, and begin_running_from_interrupt validated its
    // kind/range before consuming the task metadata. IRETQ restores A's RFLAGS.
    unsafe { arch::resume_interrupt_context(a_rsp) }
}

#[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
pub(crate) fn timer_interrupt_frame_capture_for_resume(
    frame_rsp: u64,
    frame_bytes: u64,
    hardware_rsp: u64,
    rflags: u64,
    aligned: bool,
    rsp_delta_ok: bool,
    frame_fields_ok: bool,
) {
    if !C2G_ARMED.load(Ordering::Acquire)
        || !SCHEDULER_INITIALIZED.load(Ordering::Acquire)
    {
        return;
    }

    let state = scheduler();
    if state.current.load(Ordering::Acquire) != CURRENT_A
        || C2G_CAPTURED.load(Ordering::Acquire)
    {
        return;
    }

    let task_a = state.task_a();
    let info = task_a.info();
    let hardware_rsp_in_stack =
        hardware_rsp >= info.stack_bottom && hardware_rsp <= info.stack_top;

    assert!(aligned, "C2g timer frame call boundary lost 16-byte alignment");
    assert!(rsp_delta_ok, "C2g interrupted RSP delta is not 160/168");
    assert!(frame_fields_ok, "C2g timer Rust-frame field layout mismatch");
    assert!(hardware_rsp_in_stack, "C2g hardware return RSP escaped A stack");
    assert!(rflags & (1 << 9) != 0, "C2g interrupted A with IF clear");

    task_a
        .observe_interrupt_context(frame_rsp, frame_bytes)
        .expect("C2g failed to publish A Interrupt frame");
    assert_eq!(task_a.saved_context_kind(), SavedContextKind::Interrupt);
    assert!(task_a.saved_stack_pointer_in_stack());

    C2G_FRAME_ADDRESS.store(frame_rsp, Ordering::Release);
    C2G_PROBE_HITS.fetch_add(1, Ordering::AcqRel);
    C2G_CAPTURED.store(true, Ordering::Release);
}

#[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
pub(crate) fn consume_timer_interrupt_resume_handoff() -> bool {
    C2G_CAPTURED.swap(false, Ordering::AcqRel)
}

#[cfg(feature = "m35c2g-ci-resume-interrupt-test")]
pub(crate) fn timer_interrupt_fixed_handoff_to_b() -> ! {
    let state = scheduler();
    let task_a = state.task_a();
    let task_b = state.task_b();

    assert!(!arch::in_interrupt());
    assert!(!arch::interrupts_enabled());
    assert_eq!(state.current.load(Ordering::Acquire), CURRENT_A);
    assert_eq!(task_a.saved_context_kind(), SavedContextKind::Interrupt);
    assert!(task_a.saved_stack_pointer_in_stack());
    assert_eq!(task_b.saved_context_kind(), SavedContextKind::Voluntary);

    let b_rsp = task_b.saved_stack_pointer();
    assert!(task_b.saved_stack_pointer_in_stack());

    task_a
        .park_interrupt_context()
        .expect("C2g failed to park A Interrupt context as Runnable");
    task_b
        .begin_running_from_saved()
        .expect("C2g failed to enter B from its Voluntary frame");
    state.current.store(CURRENT_B, Ordering::Release);

    // SAFETY: B's voluntary frame was validated under IF=0 and B's stack is
    // scheduler-owned. The existing entry path enables IF only on B's stack.
    unsafe { arch::start_first_task(b_rsp) }
}


#[cfg(feature = "m35c2i-ci-preempt-test")]
pub fn ci_timer_round_robin_test() -> ! {
    let task_a_ref = object::create_task_ref()
        .expect("C2i failed to create scheduler-owned task A");
    let task_b_ref = object::create_task_ref()
        .expect("C2i failed to create scheduler-owned task B");

    let task_a_handle = object::install_handle_for_ref(&task_a_ref, Rights::READ)
        .expect("C2i failed to install task A bootstrap handle");
    let task_b_handle = object::install_handle_for_ref(&task_b_ref, Rights::READ)
        .expect("C2i failed to install task B bootstrap handle");
    object::close(task_a_handle)
        .expect("C2i failed to close task A bootstrap handle");
    object::close(task_b_handle)
        .expect("C2i failed to close task B bootstrap handle");

    object::task_from_ref(&task_a_ref)
        .expect("C2i task A reference changed type")
        .prepare_initial_context(task_preempt_a_entry)
        .expect("C2i failed to prepare task A initial frame");
    object::task_from_ref(&task_b_ref)
        .expect("C2i task B reference changed type")
        .prepare_initial_context(task_preempt_b_entry)
        .expect("C2i failed to prepare task B initial frame");

    install(task_a_ref, task_b_ref);

    let state = scheduler();
    assert_eq!(Arc::strong_count(&state.task_a), 1);
    assert_eq!(Arc::strong_count(&state.task_b), 1);

    let task_a = state.task_a();
    let first_rsp = task_a.saved_stack_pointer();
    assert_eq!(task_a.saved_context_kind(), SavedContextKind::Voluntary);
    assert!(task_a.saved_stack_pointer_in_stack());
    assert!(arch::interrupts_enabled());

    crate::arch::serial::println(
        "FreeWorldOS: M3.5-C2i preemption: starting two-task one-tick round-robin",
    );

    arch::disable_interrupts();
    task_a
        .begin_running_from_saved()
        .expect("C2i task A could not transition Runnable -> Running");
    state.current.store(CURRENT_A, Ordering::Release);

    // SAFETY: A owns a valid Voluntary entry frame and the scheduler owns both
    // task lifetimes. start_first_task enables IF only on A's live task stack.
    unsafe { arch::start_first_task(first_rsp) }
}

#[cfg(feature = "m35c2i-ci-preempt-test")]
extern "C" fn task_preempt_a_entry() -> ! {
    task_preempt_loop(CURRENT_A)
}

#[cfg(feature = "m35c2i-ci-preempt-test")]
extern "C" fn task_preempt_b_entry() -> ! {
    task_preempt_loop(CURRENT_B)
}

#[cfg(feature = "m35c2i-ci-preempt-test")]
fn task_preempt_loop(task_id: u8) -> ! {
    loop {
        let state = scheduler();
        assert_eq!(state.current.load(Ordering::Acquire), task_id);
        assert!(!arch::in_interrupt());
        assert!(arch::interrupts_enabled());

        match task_id {
            CURRENT_A => {
                C2I_A_RUNS.fetch_add(1, Ordering::AcqRel);
                assert_eq!(state.task_a().state(), TaskState::Running);
                assert_eq!(state.task_a().saved_context_kind(), SavedContextKind::None);
            }
            CURRENT_B => {
                C2I_B_RUNS.fetch_add(1, Ordering::AcqRel);
                assert_eq!(state.task_b().state(), TaskState::Running);
                assert_eq!(state.task_b().saved_context_kind(), SavedContextKind::None);
            }
            _ => panic!("C2i invalid running task id"),
        }

        if C2I_SWITCHES.load(Ordering::Acquire) >= 6 {
            arch::disable_interrupts();

            let current = state.current.load(Ordering::Acquire);
            let (running, parked) = if current == CURRENT_A {
                (state.task_a(), state.task_b())
            } else {
                (state.task_b(), state.task_a())
            };

            assert_eq!(running.state(), TaskState::Running);
            assert_eq!(running.saved_context_kind(), SavedContextKind::None);
            assert!(!running.saved_stack_pointer_present());

            assert_eq!(parked.state(), TaskState::Runnable);
            assert_eq!(parked.saved_context_kind(), SavedContextKind::Interrupt);
            assert!(parked.saved_stack_pointer_present());
            assert!(parked.saved_stack_pointer_in_stack());

            let switches = C2I_SWITCHES.load(Ordering::Acquire);
            let a_to_b = C2I_A_TO_B.load(Ordering::Acquire);
            let b_to_a = C2I_B_TO_A.load(Ordering::Acquire);
            let a_runs = C2I_A_RUNS.load(Ordering::Acquire);
            let b_runs = C2I_B_RUNS.load(Ordering::Acquire);
            let voluntary = C2I_VOLUNTARY_STARTS.load(Ordering::Acquire);
            let interrupt = C2I_INTERRUPT_RESUMES.load(Ordering::Acquire);

            assert!(switches >= 6);
            assert!(a_to_b >= 3);
            assert!(b_to_a >= 3);
            assert!(a_runs >= 2);
            assert!(b_runs >= 2);
            assert_eq!(voluntary, 1);
            assert!(interrupt >= 5);
            assert!(!arch::in_interrupt());

            crate::arch::serial::write_fmt(format_args!(
                "FreeWorldOS: M3.5-C2i timer round-robin: switches={} A_to_B={} B_to_A={} A_runs={} B_runs={} voluntary_starts={} interrupt_resumes={} quantum_ticks=1 policy=round_robin depth=clear eoi_before_switch=ok\n",
                switches,
                a_to_b,
                b_to_a,
                a_runs,
                b_runs,
                voluntary,
                interrupt,
            ));
            crate::arch::serial::println(
                "FreeWorldOS: M3.5-C2i preemption proof: passed timer_selected=ok two_task_round_robin=ok priorities=off smp=off",
            );

            arch::halt_loop()
        }

        // SAFETY: IF is enabled. The periodic timer preempts this task from its
        // own scheduler-owned stack and may resume a different task.
        unsafe {
            core::arch::asm!("hlt", options(nostack, preserves_flags));
        }
    }
}

#[cfg(feature = "m35c2i-ci-preempt-test")]
pub(crate) fn timer_preemption_capture(
    frame_rsp: u64,
    frame_bytes: u64,
    hardware_rsp: u64,
    rflags: u64,
    aligned: bool,
) -> bool {
    if !SCHEDULER_INITIALIZED.load(Ordering::Acquire) {
        return false;
    }

    let state = scheduler();
    let current = state.current.load(Ordering::Acquire);
    let (outgoing, incoming) = match current {
        CURRENT_A => (state.task_a(), state.task_b()),
        CURRENT_B => (state.task_b(), state.task_a()),
        _ => return false,
    };

    if incoming.state() != TaskState::Runnable
        || !incoming.saved_stack_pointer_present()
        || !incoming.saved_stack_pointer_in_stack()
        || incoming.saved_context_kind() == SavedContextKind::None
    {
        return false;
    }

    let info = outgoing.info();
    let hardware_rsp_in_stack =
        hardware_rsp >= info.stack_bottom && hardware_rsp <= info.stack_top;

    assert!(aligned, "C2i timer frame call boundary lost 16-byte alignment");
    assert!(hardware_rsp_in_stack, "C2i interrupted RSP escaped current task stack");
    assert!(rflags & (1 << 9) != 0, "C2i timer interrupted a task with IF clear");

    outgoing
        .observe_interrupt_context(frame_rsp, frame_bytes)
        .expect("C2i failed to publish outgoing Interrupt frame");
    assert_eq!(outgoing.saved_context_kind(), SavedContextKind::Interrupt);
    assert!(outgoing.saved_stack_pointer_in_stack());

    true
}

#[cfg(feature = "m35c2i-ci-preempt-test")]
pub(crate) fn timer_preemption_handoff() -> ! {
    let state = scheduler();
    assert!(!arch::in_interrupt());
    assert!(!arch::interrupts_enabled());

    let current = state.current.load(Ordering::Acquire);
    let (outgoing, incoming, next_id) = match current {
        CURRENT_A => (state.task_a(), state.task_b(), CURRENT_B),
        CURRENT_B => (state.task_b(), state.task_a(), CURRENT_A),
        _ => panic!("C2i timer handoff without a current task"),
    };

    assert_eq!(outgoing.state(), TaskState::Running);
    assert_eq!(outgoing.saved_context_kind(), SavedContextKind::Interrupt);
    assert!(outgoing.saved_stack_pointer_in_stack());

    assert_eq!(incoming.state(), TaskState::Runnable);
    assert!(incoming.saved_stack_pointer_present());
    assert!(incoming.saved_stack_pointer_in_stack());

    let next_rsp = incoming.saved_stack_pointer();
    let next_kind = incoming.saved_context_kind();

    outgoing
        .park_interrupt_context()
        .expect("C2i failed to park outgoing Interrupt context");

    match next_kind {
        SavedContextKind::Voluntary => {
            incoming
                .begin_running_from_saved()
                .expect("C2i failed to start incoming Voluntary context");
            C2I_VOLUNTARY_STARTS.fetch_add(1, Ordering::AcqRel);
        }
        SavedContextKind::Interrupt => {
            incoming
                .begin_running_from_interrupt()
                .expect("C2i failed to resume incoming Interrupt context");
            C2I_INTERRUPT_RESUMES.fetch_add(1, Ordering::AcqRel);
        }
        SavedContextKind::None => panic!("C2i selected runnable task without saved context"),
    }

    state.current.store(next_id, Ordering::Release);
    C2I_SWITCHES.fetch_add(1, Ordering::AcqRel);
    if current == CURRENT_A {
        C2I_A_TO_B.fetch_add(1, Ordering::AcqRel);
    } else {
        C2I_B_TO_A.fetch_add(1, Ordering::AcqRel);
    }

    match next_kind {
        SavedContextKind::Voluntary => {
            // SAFETY: IF is clear, next_rsp was validated on the scheduler-owned
            // target stack, and the Voluntary entry path enables IF after the
            // target stack/register frame is active.
            unsafe { arch::start_first_task(next_rsp) }
        }
        SavedContextKind::Interrupt => {
            // SAFETY: IF is clear, next_rsp points at the validated 160-byte
            // Interrupt frame, and IRETQ restores that task's saved RFLAGS.
            unsafe { arch::resume_interrupt_context(next_rsp) }
        }
        SavedContextKind::None => unreachable!(),
    }
}


const RUN_QUEUE_CAPACITY: usize = 8;

struct RunQueue {
    slots: [Option<ObjectRef>; RUN_QUEUE_CAPACITY],
    cleanup: [Option<ObjectRef>; RUN_QUEUE_CAPACITY],
    idle: ObjectRef,
    current: Option<usize>,
}

impl RunQueue {
    fn new(idle: ObjectRef) -> Self {
        Self {
            slots: core::array::from_fn(|_| None),
            cleanup: core::array::from_fn(|_| None),
            idle,
            current: None,
        }
    }

    fn enqueue(&mut self, object: ObjectRef) -> usize {
        assert!(
            !arch::interrupts_enabled(),
            "FreeWorld run-queue mutation requires interrupts off"
        );
        assert!(
            self.slots
                .iter()
                .flatten()
                .all(|existing| !Arc::ptr_eq(existing, &object)),
            "FreeWorld attempted to enqueue the same task twice"
        );

        let index = self
            .slots
            .iter()
            .position(Option::is_none)
            .expect("FreeWorld run queue capacity exhausted");
        self.slots[index] = Some(object);
        index
    }

    fn task_at(&self, index: usize) -> &TaskObject {
        object::task_from_ref(
            self.slots[index]
                .as_ref()
                .expect("FreeWorld run-queue slot unexpectedly empty"),
        )
        .expect("FreeWorld run-queue entry changed object type")
    }

    fn idle_task(&self) -> &TaskObject {
        object::task_from_ref(&self.idle)
            .expect("FreeWorld idle reference changed object type")
    }

    fn current_task(&self) -> &TaskObject {
        match self.current {
            Some(index) => self.task_at(index),
            None => self.idle_task(),
        }
    }

    fn next_runnable_after(&self, current: Option<usize>) -> Option<usize> {
        let start = current.map_or(0, |index| (index + 1) % RUN_QUEUE_CAPACITY);

        for offset in 0..RUN_QUEUE_CAPACITY {
            let index = (start + offset) % RUN_QUEUE_CAPACITY;
            let Some(object) = self.slots[index].as_ref() else {
                continue;
            };
            let task = object::task_from_ref(object)
                .expect("FreeWorld run-queue entry changed object type");

            if task.state() == TaskState::Runnable
                && task.saved_stack_pointer_present()
                && task.saved_stack_pointer_in_stack()
                && task.saved_context_kind() != SavedContextKind::None
            {
                return Some(index);
            }
        }

        None
    }

    fn defer_cleanup(&mut self, object: ObjectRef) {
        assert!(
            !arch::interrupts_enabled(),
            "FreeWorld deferred cleanup mutation requires interrupts off"
        );
        let slot = self
            .cleanup
            .iter_mut()
            .find(|slot| slot.is_none())
            .expect("FreeWorld deferred cleanup capacity exhausted");
        *slot = Some(object);
    }

    fn drain_cleanup(&mut self) -> u64 {
        assert!(
            !arch::interrupts_enabled(),
            "FreeWorld cleanup requires interrupts off"
        );

        let mut cleaned = 0;
        for slot in &mut self.cleanup {
            if let Some(object) = slot.take() {
                drop(object);
                cleaned += 1;
            }
        }
        cleaned
    }

    fn runnable_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }
}

struct RunQueueStorage(UnsafeCell<MaybeUninit<RunQueue>>);

unsafe impl Sync for RunQueueStorage {}

static RUN_QUEUE: RunQueueStorage =
    RunQueueStorage(UnsafeCell::new(MaybeUninit::uninit()));
static RUN_QUEUE_INITIALIZED: AtomicBool = AtomicBool::new(false);

static DEFAULT_KERNEL_TASK_RAN: AtomicBool = AtomicBool::new(false);
static DEFAULT_KERNEL_TASK_RECLAIMED: AtomicBool = AtomicBool::new(false);
static DEFAULT_KERNEL_TASK_ID: AtomicU64 = AtomicU64::new(0);
static DEFAULT_KERNEL_TASK_SLOT: AtomicU64 = AtomicU64::new(u64::MAX);


pub(crate) fn enqueue_prepared_task(object: ObjectRef) -> usize {
    assert!(
        !arch::interrupts_enabled(),
        "FreeWorld enqueue_prepared_task requires interrupts off"
    );
    let task = object::task_from_ref(&object)
        .expect("FreeWorld enqueue object changed type");
    assert_eq!(task.state(), TaskState::Runnable);
    assert!(task.saved_stack_pointer_present());
    assert!(task.saved_stack_pointer_in_stack());
    assert_ne!(task.saved_context_kind(), SavedContextKind::None);
    run_queue_mut().enqueue(object)
}

pub(crate) fn preemption_disable_current() {
    assert!(arch::interrupts_enabled());
    arch::disable_interrupts();
    run_queue_mut()
        .current_task()
        .preemption_disable()
        .expect("FreeWorld could not enter preemption-disabled region");
    arch::enable_interrupts();
}

pub(crate) fn preemption_enable_current() {
    assert!(arch::interrupts_enabled());
    arch::disable_interrupts();
    run_queue_mut()
        .current_task()
        .preemption_enable()
        .expect("FreeWorld could not leave preemption-disabled region");
    arch::enable_interrupts();
}

fn drain_cleanup_from_live_stack() -> u64 {
    assert!(
        arch::interrupts_enabled(),
        "FreeWorld cleanup entry expects IF enabled"
    );

    arch::disable_interrupts();
    let cleaned = run_queue_mut().drain_cleanup();
    arch::enable_interrupts();
    cleaned
}

pub(crate) fn exit_current() -> ! {
    exit_current_expected(None)
}

fn exit_current_expected(expected_slot: Option<usize>) -> ! {
    assert!(arch::interrupts_enabled());
    arch::disable_interrupts();

    let queue = run_queue_mut();
    let slot = queue.current.expect("FreeWorld idle task cannot exit via exit_current");
    if let Some(expected_slot) = expected_slot {
        assert_eq!(
            slot,
            expected_slot,
            "FreeWorld exit_current task/slot identity changed before removal"
        );
    }

    let object_ref = queue.slots[slot]
        .take()
        .expect("FreeWorld exiting task missing from run queue");
    let exiting = object::task_from_ref(&object_ref)
        .expect("FreeWorld exiting run-queue entry changed object type");
    exiting
        .stop_running_without_saved_context()
        .expect("FreeWorld exiting task was not safely stoppable");

    queue.defer_cleanup(object_ref);

    #[cfg(feature = "m35c2j-ci-run-queue-test")]
    C2J_EXITS.fetch_add(1, Ordering::AcqRel);

    if let Some(next_slot) = queue.next_runnable_after(Some(slot)) {
        let incoming = queue.task_at(next_slot);
        let next_rsp = incoming.saved_stack_pointer();
        let next_kind = incoming.saved_context_kind();

        match next_kind {
            SavedContextKind::Voluntary => incoming
                .begin_running_from_saved()
                .expect("FreeWorld exit path could not start Voluntary task"),
            SavedContextKind::Interrupt => incoming
                .begin_running_from_interrupt()
                .expect("FreeWorld exit path could not resume Interrupt task"),
            SavedContextKind::None => panic!("FreeWorld exit path selected task without context"),
        }

        queue.current = Some(next_slot);

        match next_kind {
            SavedContextKind::Voluntary => unsafe { arch::start_first_task(next_rsp) },
            SavedContextKind::Interrupt => unsafe { arch::resume_interrupt_context(next_rsp) },
            SavedContextKind::None => unreachable!(),
        }
    }

    let idle = queue.idle_task();
    let idle_rsp = idle.saved_stack_pointer();
    let idle_kind = idle.saved_context_kind();

    match idle_kind {
        SavedContextKind::Voluntary => idle
            .begin_running_from_saved()
            .expect("FreeWorld exit path could not start idle task"),
        SavedContextKind::Interrupt => idle
            .begin_running_from_interrupt()
            .expect("FreeWorld exit path could not resume idle task"),
        SavedContextKind::None => panic!("FreeWorld idle task has no saved context"),
    }
    queue.current = None;

    match idle_kind {
        SavedContextKind::Voluntary => unsafe { arch::start_first_task(idle_rsp) },
        SavedContextKind::Interrupt => unsafe { arch::resume_interrupt_context(idle_rsp) },
        SavedContextKind::None => unreachable!(),
    }
}


pub fn start_default_scheduler() -> ! {
    assert!(
        !RUN_QUEUE_INITIALIZED.load(Ordering::Acquire),
        "FreeWorld default scheduler already initialized"
    );

    let idle = object::create_task_ref()
        .expect("FreeWorld failed to create default idle task");
    object::task_from_ref(&idle)
        .expect("FreeWorld idle reference changed type")
        .prepare_initial_context(default_idle_entry)
        .expect("FreeWorld failed to prepare default idle task");

    let worker = object::create_task_ref()
        .expect("FreeWorld failed to create default kernel task");
    let worker_task = object::task_from_ref(&worker)
        .expect("FreeWorld default kernel task reference changed type");
    worker_task
        .prepare_initial_context(default_kernel_task_entry)
        .expect("FreeWorld failed to prepare default kernel task");
    let worker_id = worker_task.info().id;
    DEFAULT_KERNEL_TASK_ID.store(worker_id, Ordering::Release);

    assert!(
        arch::interrupts_enabled(),
        "FreeWorld default scheduler expects IF enabled before handoff"
    );

    arch::disable_interrupts();
    install_run_queue(idle);

    let queue = run_queue_mut();
    let worker_slot = queue.enqueue(worker);
    DEFAULT_KERNEL_TASK_SLOT.store(worker_slot as u64, Ordering::Release);

    let idle = queue.idle_task();
    let idle_rsp = idle.saved_stack_pointer();
    let idle_id = idle.info().id;
    idle
        .begin_running_from_saved()
        .expect("FreeWorld failed to start default idle task");
    queue.current = None;

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M3.5-C2k default scheduler: queue=online capacity={} idle_task={} IF=handoff\n",
        RUN_QUEUE_CAPACITY,
        idle_id,
    ));
    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M3.5-C2l kernel task: queued task_id={} slot={} state=runnable dispatch=timer\n",
        worker_id,
        worker_slot,
    ));

    // SAFETY: The scheduler owns idle and the queued worker. Idle has a
    // validated Voluntary frame on its higher-half task stack, and IF remains
    // clear until start_first_task restores that stack and executes STI.
    unsafe { arch::start_first_task(idle_rsp) }
}

extern "C" fn default_kernel_task_entry() -> ! {
    assert!(arch::interrupts_enabled());

    arch::disable_interrupts();
    let queue = run_queue_mut();
    let slot = queue
        .current
        .expect("FreeWorld default kernel task entered while idle was current");
    let expected_slot = DEFAULT_KERNEL_TASK_SLOT.load(Ordering::Acquire) as usize;
    assert_eq!(slot, expected_slot);

    let worker = queue.task_at(slot);
    let worker_id = worker.info().id;
    assert_eq!(worker_id, DEFAULT_KERNEL_TASK_ID.load(Ordering::Acquire));
    assert_eq!(worker.state(), TaskState::Running);
    assert_eq!(worker.saved_context_kind(), SavedContextKind::None);
    assert!(!worker.saved_stack_pointer_present());

    let idle = queue.idle_task();
    assert_eq!(idle.state(), TaskState::Runnable);
    assert_eq!(idle.saved_context_kind(), SavedContextKind::Interrupt);
    assert!(idle.saved_stack_pointer_present());
    assert!(idle.saved_stack_pointer_in_stack());
    arch::enable_interrupts();

    assert!(
        !DEFAULT_KERNEL_TASK_RAN.swap(true, Ordering::AcqRel),
        "FreeWorld default kernel task entered more than once"
    );

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M3.5-C2l kernel task: running task_id={} slot={} source=timer idle_saved=interrupt IF=on\n",
        worker_id,
        slot,
    ));
    crate::arch::serial::println(
        "FreeWorldOS: M3.5-C2l kernel task: work=complete exit=begin",
    );

    exit_current()
}

extern "C" fn default_idle_entry() -> ! {
    assert!(arch::interrupts_enabled());

    let before = super::time::now().0;
    let mut timer_return_proven = false;

    loop {
        let cleaned = drain_cleanup_from_live_stack();
        if cleaned != 0 {
            if DEFAULT_KERNEL_TASK_RAN.load(Ordering::Acquire) {
                assert_eq!(
                    cleaned,
                    1,
                    "FreeWorld default idle reclaimed an unexpected task count"
                );
                assert!(
                    !DEFAULT_KERNEL_TASK_RECLAIMED.swap(true, Ordering::AcqRel),
                    "FreeWorld default kernel task was reclaimed more than once"
                );
            }

            crate::arch::serial::write_fmt(format_args!(
                "FreeWorldOS: scheduler idle reclaimed stopped_tasks={}\n",
                cleaned,
            ));
        }

        if !timer_return_proven {
            let after = super::time::now().0;
            if after > before {
                arch::disable_interrupts();
                let queue = run_queue_mut();
                assert!(queue.current.is_none());
                assert!(queue.runnable_empty());
                assert_eq!(queue.idle_task().state(), TaskState::Running);
                assert!(DEFAULT_KERNEL_TASK_RAN.load(Ordering::Acquire));
                assert!(DEFAULT_KERNEL_TASK_RECLAIMED.load(Ordering::Acquire));
                arch::enable_interrupts();

                crate::arch::serial::write_fmt(format_args!(
                    "FreeWorldOS: M3.5-C2k default scheduler: idle=running timer_return=ok tick_before={} tick_after={} queue_empty=ok\n",
                    before,
                    after,
                ));
                crate::arch::serial::println(
                    "FreeWorldOS: M3.5-C2l proof: ordinary_kernel_task=ok timer_dispatch=ok exit=ok off_stack_reclaim=ok idle_resumed=ok priorities=off smp=off",
                );
                timer_return_proven = true;
            }
        }

        // SAFETY: IF is enabled. With no ordinary runnable tasks the timer
        // accounts/EOIs and IRETQs back to this scheduler-owned idle stack.
        unsafe {
            core::arch::asm!("hlt", options(nostack, preserves_flags));
        }
    }
}

#[cfg(feature = "m35c2j-ci-run-queue-test")]
static C2J_SWITCHES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2j-ci-run-queue-test")]
static C2J_VOLUNTARY_STARTS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2j-ci-run-queue-test")]
static C2J_INTERRUPT_RESUMES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2j-ci-run-queue-test")]
static C2J_A_RUNS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2j-ci-run-queue-test")]
static C2J_B_RUNS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2j-ci-run-queue-test")]
static C2J_C_RUNS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2j-ci-run-queue-test")]
static C2J_PREEMPT_BLOCKED_TICKS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2j-ci-run-queue-test")]
static C2J_PREEMPT_BLOCK_PROVEN: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2j-ci-run-queue-test")]
static C2J_C_EXITED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2j-ci-run-queue-test")]
static C2J_B_EXITED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "m35c2j-ci-run-queue-test")]
static C2J_EXITS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "m35c2j-ci-run-queue-test")]
static C2J_CLEANED: AtomicU64 = AtomicU64::new(0);

fn run_queue_mut() -> &'static mut RunQueue {
    assert!(
        RUN_QUEUE_INITIALIZED.load(Ordering::Acquire),
        "FreeWorld run queue used before initialization"
    );
    assert!(
        !arch::interrupts_enabled(),
        "FreeWorld run-queue access requires interrupts off"
    );

    // SAFETY: The production scheduler is bootstrap-CPU-only in M3.5-C.
    // Task-context structural access disables interrupts first, and the timer
    // path already runs with IF clear. SMP must replace this exclusion model.
    unsafe { &mut *(*RUN_QUEUE.0.get()).as_mut_ptr() }
}

fn install_run_queue(idle: ObjectRef) {
    assert!(!arch::interrupts_enabled());
    assert!(
        !RUN_QUEUE_INITIALIZED.load(Ordering::Acquire),
        "FreeWorld run queue installed twice"
    );

    // SAFETY: Single bootstrap CPU, one initialization before publication.
    unsafe {
        (*RUN_QUEUE.0.get()).write(RunQueue::new(idle));
    }
    RUN_QUEUE_INITIALIZED.store(true, Ordering::Release);
}

#[cfg(feature = "m35c2j-ci-run-queue-test")]
fn c2j_drain_cleanup_from_live_stack() {
    let cleaned = drain_cleanup_from_live_stack();
    if cleaned != 0 {
        C2J_CLEANED.fetch_add(cleaned, Ordering::AcqRel);
    }
}

#[cfg(feature = "m35c2j-ci-run-queue-test")]
fn c2j_preemption_disable_current() {
    preemption_disable_current();
}

#[cfg(feature = "m35c2j-ci-run-queue-test")]
fn c2j_preemption_enable_current() {
    preemption_enable_current();
}

#[cfg(feature = "m35c2j-ci-run-queue-test")]
pub fn ci_run_queue_test() -> ! {
    let task_a = object::create_task_ref().expect("C2j failed to create task A");
    let task_b = object::create_task_ref().expect("C2j failed to create task B");
    let task_c = object::create_task_ref().expect("C2j failed to create task C");
    let idle = object::create_task_ref().expect("C2j failed to create idle task");

    for object_ref in [&task_a, &task_b, &task_c, &idle] {
        let handle = object::install_handle_for_ref(object_ref, Rights::READ)
            .expect("C2j failed to install bootstrap task handle");
        object::close(handle).expect("C2j failed to close bootstrap task handle");
    }

    object::task_from_ref(&task_a)
        .expect("C2j task A reference changed type")
        .prepare_initial_context(c2j_task_a_entry)
        .expect("C2j failed to prepare task A");
    object::task_from_ref(&task_b)
        .expect("C2j task B reference changed type")
        .prepare_initial_context(c2j_task_b_entry)
        .expect("C2j failed to prepare task B");
    object::task_from_ref(&task_c)
        .expect("C2j task C reference changed type")
        .prepare_initial_context(c2j_task_c_entry)
        .expect("C2j failed to prepare task C");
    object::task_from_ref(&idle)
        .expect("C2j idle reference changed type")
        .prepare_initial_context(c2j_idle_entry)
        .expect("C2j failed to prepare idle task");

    assert!(arch::interrupts_enabled());
    arch::disable_interrupts();

    install_run_queue(idle);
    let queue = run_queue_mut();
    let a_slot = queue.enqueue(task_a);
    let b_slot = queue.enqueue(task_b);
    let c_slot = queue.enqueue(task_c);
    assert_eq!((a_slot, b_slot, c_slot), (0, 1, 2));

    let first_rsp = queue.task_at(a_slot).saved_stack_pointer();
    queue
        .task_at(a_slot)
        .begin_running_from_saved()
        .expect("C2j failed to start task A");
    queue.current = Some(a_slot);

    crate::arch::serial::println(
        "FreeWorldOS: M3.5-C2j run queue: starting A->B->C rotation capacity=8",
    );

    // SAFETY: A's Voluntary frame is valid, the queue owns all task lifetimes,
    // and IF remains clear until A's task stack/register frame is active.
    unsafe { arch::start_first_task(first_rsp) }
}

#[cfg(feature = "m35c2j-ci-run-queue-test")]
extern "C" fn c2j_task_a_entry() -> ! {
    c2j_preemption_disable_current();

    let before_tick = super::time::now().0;
    let before_switches = C2J_SWITCHES.load(Ordering::Acquire);
    while super::time::now().0 == before_tick {
        // SAFETY: IF is enabled. The timer must return to A while its
        // preemption-disable depth is nonzero.
        unsafe {
            core::arch::asm!("hlt", options(nostack, preserves_flags));
        }
    }

    assert_eq!(
        C2J_SWITCHES.load(Ordering::Acquire),
        before_switches,
        "C2j timer switched tasks inside a preemption-disabled region"
    );
    assert!(
        C2J_PREEMPT_BLOCKED_TICKS.load(Ordering::Acquire) >= 1,
        "C2j timer did not observe the preemption-disable counter"
    );
    C2J_PREEMPT_BLOCK_PROVEN.store(true, Ordering::Release);
    c2j_preemption_enable_current();

    c2j_task_loop(0)
}

#[cfg(feature = "m35c2j-ci-run-queue-test")]
extern "C" fn c2j_task_b_entry() -> ! {
    c2j_task_loop(1)
}

#[cfg(feature = "m35c2j-ci-run-queue-test")]
extern "C" fn c2j_task_c_entry() -> ! {
    c2j_task_loop(2)
}

#[cfg(feature = "m35c2j-ci-run-queue-test")]
fn c2j_task_loop(slot: usize) -> ! {
    loop {
        c2j_drain_cleanup_from_live_stack();

        match slot {
            0 => {
                C2J_A_RUNS.fetch_add(1, Ordering::AcqRel);
                if C2J_C_EXITED.load(Ordering::Acquire)
                    && C2J_B_EXITED.load(Ordering::Acquire)
                {
                    c2j_exit_current(slot);
                }
            }
            1 => {
                C2J_B_RUNS.fetch_add(1, Ordering::AcqRel);
                if C2J_C_EXITED.load(Ordering::Acquire) {
                    C2J_B_EXITED.store(true, Ordering::Release);
                    c2j_exit_current(slot);
                }
            }
            2 => {
                C2J_C_RUNS.fetch_add(1, Ordering::AcqRel);
                if C2J_SWITCHES.load(Ordering::Acquire) >= 6 {
                    C2J_C_EXITED.store(true, Ordering::Release);
                    c2j_exit_current(slot);
                }
            }
            _ => panic!("C2j invalid task slot"),
        }

        assert!(arch::interrupts_enabled());
        // SAFETY: The timer may preempt and rotate to another queue member.
        unsafe {
            core::arch::asm!("hlt", options(nostack, preserves_flags));
        }
    }
}

#[cfg(feature = "m35c2j-ci-run-queue-test")]
fn c2j_exit_current(slot: usize) -> ! {
    exit_current_expected(Some(slot))
}

#[cfg(feature = "m35c2j-ci-run-queue-test")]
extern "C" fn c2j_idle_entry() -> ! {
    c2j_drain_cleanup_from_live_stack();

    assert!(arch::interrupts_enabled());
    arch::disable_interrupts();
    let queue = run_queue_mut();

    assert!(queue.current.is_none());
    assert!(queue.runnable_empty());
    assert_eq!(queue.idle_task().state(), TaskState::Running);
    assert_eq!(C2J_EXITS.load(Ordering::Acquire), 3);
    assert_eq!(C2J_CLEANED.load(Ordering::Acquire), 3);
    assert!(C2J_PREEMPT_BLOCK_PROVEN.load(Ordering::Acquire));
    assert!(C2J_SWITCHES.load(Ordering::Acquire) >= 6);
    assert!(C2J_A_RUNS.load(Ordering::Acquire) >= 2);
    assert!(C2J_B_RUNS.load(Ordering::Acquire) >= 2);
    assert!(C2J_C_RUNS.load(Ordering::Acquire) >= 2);
    assert_eq!(C2J_VOLUNTARY_STARTS.load(Ordering::Acquire), 2);
    assert!(C2J_INTERRUPT_RESUMES.load(Ordering::Acquire) >= 4);
    assert!(!arch::in_interrupt());

    crate::arch::serial::write_fmt(format_args!(
        "FreeWorldOS: M3.5-C2j run queue: rotation=A>B>C>A switches={} A_runs={} B_runs={} C_runs={} preempt_blocked_ticks={} exits={} cleaned={} idle=running queue=empty capacity=8 mutation_if=off\n",
        C2J_SWITCHES.load(Ordering::Acquire),
        C2J_A_RUNS.load(Ordering::Acquire),
        C2J_B_RUNS.load(Ordering::Acquire),
        C2J_C_RUNS.load(Ordering::Acquire),
        C2J_PREEMPT_BLOCKED_TICKS.load(Ordering::Acquire),
        C2J_EXITS.load(Ordering::Acquire),
        C2J_CLEANED.load(Ordering::Acquire),
    ));
    crate::arch::serial::println(
        "FreeWorldOS: M3.5-C2j proof: passed variable_queue=ok three_tasks=ok preempt_disable=ok deferred_cleanup=ok idle=ok priorities=off smp=off",
    );

    arch::halt_loop()
}

pub(crate) fn timer_run_queue_capture(
    frame_rsp: u64,
    frame_bytes: u64,
    hardware_rsp: u64,
    rflags: u64,
    aligned: bool,
) -> bool {
    if !RUN_QUEUE_INITIALIZED.load(Ordering::Acquire) {
        return false;
    }

    let queue = run_queue_mut();
    let outgoing = queue.current_task();

    if outgoing.preemption_disabled() {
        #[cfg(feature = "m35c2j-ci-run-queue-test")]
        C2J_PREEMPT_BLOCKED_TICKS.fetch_add(1, Ordering::AcqRel);
        return false;
    }

    if queue.next_runnable_after(queue.current).is_none() {
        return false;
    }

    let info = outgoing.info();
    let hardware_rsp_in_stack =
        hardware_rsp >= info.stack_bottom && hardware_rsp <= info.stack_top;

    assert!(aligned, "FreeWorld timer frame call boundary lost 16-byte alignment");
    assert!(hardware_rsp_in_stack, "FreeWorld interrupted RSP escaped current stack");
    assert!(rflags & (1 << 9) != 0, "FreeWorld timer interrupted task with IF clear");

    outgoing
        .observe_interrupt_context(frame_rsp, frame_bytes)
        .expect("FreeWorld failed to publish outgoing Interrupt frame");
    assert!(outgoing.saved_stack_pointer_in_stack());
    true
}

pub(crate) fn timer_run_queue_handoff() -> ! {
    assert!(!arch::in_interrupt());
    assert!(!arch::interrupts_enabled());

    let queue = run_queue_mut();
    let old_current = queue.current;
    let outgoing = queue.current_task();

    assert_eq!(outgoing.state(), TaskState::Running);
    assert_eq!(outgoing.saved_context_kind(), SavedContextKind::Interrupt);
    assert!(outgoing.saved_stack_pointer_in_stack());

    let next_slot = queue
        .next_runnable_after(old_current)
        .expect("FreeWorld handoff lost its runnable target");
    let incoming = queue.task_at(next_slot);
    let next_rsp = incoming.saved_stack_pointer();
    let next_kind = incoming.saved_context_kind();

    #[cfg(feature = "m35c2j-ci-run-queue-test")]
    if C2J_SWITCHES.load(Ordering::Acquire) < 6 {
        let switch = C2J_SWITCHES.load(Ordering::Acquire) as usize;
        let expected = [
            (Some(0), 1usize),
            (Some(1), 2usize),
            (Some(2), 0usize),
            (Some(0), 1usize),
            (Some(1), 2usize),
            (Some(2), 0usize),
        ][switch];
        assert_eq!(
            (old_current, next_slot),
            expected,
            "C2j first six timer selections were not A->B->C->A"
        );
    }

    outgoing
        .park_interrupt_context()
        .expect("FreeWorld failed to park outgoing Interrupt context");

    match next_kind {
        SavedContextKind::Voluntary => {
            incoming
                .begin_running_from_saved()
                .expect("FreeWorld failed to start incoming Voluntary context");
            #[cfg(feature = "m35c2j-ci-run-queue-test")]
            C2J_VOLUNTARY_STARTS.fetch_add(1, Ordering::AcqRel);
        }
        SavedContextKind::Interrupt => {
            incoming
                .begin_running_from_interrupt()
                .expect("FreeWorld failed to resume incoming Interrupt context");
            #[cfg(feature = "m35c2j-ci-run-queue-test")]
            C2J_INTERRUPT_RESUMES.fetch_add(1, Ordering::AcqRel);
        }
        SavedContextKind::None => panic!("FreeWorld selected runnable task without saved context"),
    }

    queue.current = Some(next_slot);
    #[cfg(feature = "m35c2j-ci-run-queue-test")]
    C2J_SWITCHES.fetch_add(1, Ordering::AcqRel);

    match next_kind {
        SavedContextKind::Voluntary => {
            // SAFETY: target frame and stack were validated under IF=0.
            unsafe { arch::start_first_task(next_rsp) }
        }
        SavedContextKind::Interrupt => {
            // SAFETY: target Interrupt frame is scheduler-owned and validated.
            unsafe { arch::resume_interrupt_context(next_rsp) }
        }
        SavedContextKind::None => unreachable!(),
    }
}

//! End-to-end FIFO scheduling diagnostic.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use trains_primitives::task::{Priority, TaskId};

const TRACE_LENGTH: usize = 6;
const TASK_A_THREAD_POINTER: u64 = 0xaaaa;
const TASK_B_THREAD_POINTER: u64 = 0xbbbb;

static TRACE_INDEX: AtomicUsize = AtomicUsize::new(0);
static TRACE: [AtomicUsize; TRACE_LENGTH] = [const { AtomicUsize::new(0) }; TRACE_LENGTH];
static THREAD_POINTERS_VALID: AtomicBool = AtomicBool::new(true);

fn set_thread_pointer(value: u64) {
    // SAFETY: TPIDR_EL0 is the calling task's writable thread-pointer register.
    unsafe {
        asm!("msr tpidr_el0, {value}", value = in(reg) value, options(nomem, nostack, preserves_flags))
    };
}

fn validate_thread_pointer(expected: u64) {
    let actual: u64;
    // SAFETY: reading TPIDR_EL0 has no side effects.
    unsafe {
        asm!("mrs {actual}, tpidr_el0", actual = out(reg) actual, options(nomem, nostack, preserves_flags))
    };
    if actual != expected {
        THREAD_POINTERS_VALID.store(false, Ordering::Relaxed);
    }
}

fn record(value: usize) {
    let index = TRACE_INDEX.fetch_add(1, Ordering::Relaxed);
    if index < TRACE_LENGTH {
        TRACE[index].store(value, Ordering::Relaxed);
    }
}

extern "C" fn task_a(_id: TaskId) -> ! {
    set_thread_pointer(TASK_A_THREAD_POINTER);
    for _ in 0..3 {
        record(1);
        crate::syscall::yield_now();
        validate_thread_pointer(TASK_A_THREAD_POINTER);
    }
    crate::syscall::exit();
}

extern "C" fn task_b(_id: TaskId) -> ! {
    set_thread_pointer(TASK_B_THREAD_POINTER);
    for _ in 0..3 {
        record(2);
        crate::syscall::yield_now();
        validate_thread_pointer(TASK_B_THREAD_POINTER);
    }
    crate::syscall::exit();
}

pub(crate) fn run() -> bool {
    TRACE_INDEX.store(0, Ordering::Relaxed);
    THREAD_POINTERS_VALID.store(true, Ordering::Relaxed);
    for value in &TRACE {
        value.store(0, Ordering::Relaxed);
    }

    let priority = Priority::new(1);
    if crate::scheduler::SCHEDULER
        .create(task_a, priority)
        .is_err()
        || crate::scheduler::SCHEDULER
            .create(task_b, priority)
            .is_err()
    {
        return false;
    }

    let Ok(outcome) = crate::scheduler::SCHEDULER.run() else {
        return false;
    };
    let expected = [1, 2, 1, 2, 1, 2];

    outcome.exited_tasks() == 2
        && THREAD_POINTERS_VALID.load(Ordering::Relaxed)
        && TRACE_INDEX.load(Ordering::Relaxed) == TRACE_LENGTH
        && TRACE
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual.load(Ordering::Relaxed) == expected)
}

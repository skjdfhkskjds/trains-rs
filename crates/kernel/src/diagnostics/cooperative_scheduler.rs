//! End-to-end equal-priority FIFO scheduling diagnostic.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use trains_primitives::task::{Priority, TaskId};

use crate::CurrentTask;
use crate::scheduler::Scheduler;

const TRACE_LENGTH: usize = 6;
const TASK_A_THREAD_POINTER: u64 = 0xaaaa;
const TASK_B_THREAD_POINTER: u64 = 0xbbbb;

/// Owns the state and task entrypoints for the scheduler boot diagnostic.
pub(crate) struct CooperativeSchedulerDiagnostic {
    trace_index: AtomicUsize,
    trace: [AtomicUsize; TRACE_LENGTH],
    thread_pointers_valid: AtomicBool,
}

pub(crate) static COOPERATIVE_SCHEDULER: CooperativeSchedulerDiagnostic =
    CooperativeSchedulerDiagnostic::new();

impl CooperativeSchedulerDiagnostic {
    const fn new() -> Self {
        Self {
            trace_index: AtomicUsize::new(0),
            trace: [const { AtomicUsize::new(0) }; TRACE_LENGTH],
            thread_pointers_valid: AtomicBool::new(true),
        }
    }

    fn set_thread_pointer(&self, value: u64) {
        // SAFETY: TPIDR_EL0 is the calling task's writable thread pointer.
        unsafe {
            asm!("msr tpidr_el0, {value}", value = in(reg) value, options(nomem, nostack, preserves_flags))
        };
    }

    fn validate_thread_pointer(&self, expected: u64) {
        let actual: u64;
        // SAFETY: reading TPIDR_EL0 has no side effects.
        unsafe {
            asm!("mrs {actual}, tpidr_el0", actual = out(reg) actual, options(nomem, nostack, preserves_flags))
        };
        if actual != expected {
            self.thread_pointers_valid.store(false, Ordering::Relaxed);
        }
    }

    fn record(&self, value: usize) {
        let index = self.trace_index.fetch_add(1, Ordering::Relaxed);
        if index < TRACE_LENGTH {
            self.trace[index].store(value, Ordering::Relaxed);
        }
    }

    extern "C" fn task_a(_id: TaskId) -> ! {
        COOPERATIVE_SCHEDULER.run_task(TASK_A_THREAD_POINTER, 1)
    }

    extern "C" fn task_b(_id: TaskId) -> ! {
        COOPERATIVE_SCHEDULER.run_task(TASK_B_THREAD_POINTER, 2)
    }

    fn run_task(&self, thread_pointer: u64, trace_value: usize) -> ! {
        self.set_thread_pointer(thread_pointer);
        for _ in 0..3 {
            self.record(trace_value);
            CurrentTask::yield_now();
            self.validate_thread_pointer(thread_pointer);
        }
        CurrentTask::exit();
    }

    pub(crate) fn run(&self, scheduler: &Scheduler) -> bool {
        self.trace_index.store(0, Ordering::Relaxed);
        self.thread_pointers_valid.store(true, Ordering::Relaxed);
        for value in &self.trace {
            value.store(0, Ordering::Relaxed);
        }

        let priority = Priority::new(1);
        if scheduler.create(Self::task_a, priority).is_err()
            || scheduler.create(Self::task_b, priority).is_err()
        {
            return false;
        }

        let Ok(outcome) = scheduler.run() else {
            return false;
        };
        let expected = [1, 2, 1, 2, 1, 2];

        outcome.exited_tasks() == 2
            && self.thread_pointers_valid.load(Ordering::Relaxed)
            && self.trace_index.load(Ordering::Relaxed) == TRACE_LENGTH
            && self
                .trace
                .iter()
                .zip(expected)
                .all(|(actual, expected)| actual.load(Ordering::Relaxed) == expected)
    }
}

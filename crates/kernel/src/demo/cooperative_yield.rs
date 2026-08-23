//! Three-task workload demonstrating cooperative FIFO yielding.

use core::fmt::Write;

use trains_platform::{Platform as _, RASPI4};
use trains_primitives::task::TaskId;

const TASK_COUNT: usize = 3;

extern "C" fn yielding_task(id: TaskId) -> ! {
    let mut console = RASPI4.console();
    writeln!(console, "task {id}: before yield").ok();
    crate::syscall::yield_now();
    writeln!(console, "task {id}: after yield").ok();
    crate::syscall::exit();
}

pub(crate) fn run() -> bool {
    for _ in 0..TASK_COUNT {
        if crate::scheduler::create(yielding_task).is_err() {
            return false;
        }
    }

    crate::scheduler::run().is_ok_and(|outcome| outcome.exited_tasks() == TASK_COUNT)
}

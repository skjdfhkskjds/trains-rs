//! Three-task workload demonstrating cooperative priority scheduling.

use core::fmt::Write;

use trains_kernel::{Priority, TaskId, runtime::CommandResult};
use trains_platform::{Platform as _, RASPI4};

const TASK_COUNT: usize = 3;

extern "C" fn yielding_task(id: TaskId) -> ! {
    let mut console = RASPI4.console();
    let priority = Priority::new(id.get());
    writeln!(
        console,
        "task {id} (priority {}): before yield",
        priority.get()
    )
    .ok();
    trains_kernel::yield_now();
    writeln!(
        console,
        "task {id} (priority {}): after yield",
        priority.get()
    )
    .ok();
    trains_kernel::exit_task();
}

pub(crate) fn run(_arguments: &str) -> CommandResult {
    let mut console = RASPI4.console();
    writeln!(console, "trains-rs: cooperative yield example").ok();

    for index in 0..TASK_COUNT {
        let priority = Priority::new(index as u32);
        if trains_kernel::create_task(yielding_task, priority).is_err() {
            writeln!(console, "trains-rs: cooperative yield example failed").ok();
            return CommandResult::Failure;
        }
    }

    if trains_kernel::run_tasks().is_ok_and(|outcome| outcome.exited_tasks() == TASK_COUNT) {
        writeln!(console, "trains-rs: cooperative yield example complete").ok();
        CommandResult::Success
    } else {
        writeln!(console, "trains-rs: cooperative yield example failed").ok();
        CommandResult::Failure
    }
}

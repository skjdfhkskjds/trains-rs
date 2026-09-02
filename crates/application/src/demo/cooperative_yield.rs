//! Three-task workload demonstrating cooperative priority scheduling.

use trains_kernel::{
    CurrentTask, Kernel, Priority, TaskId,
    runtime::{Command, CommandResult},
};
use trains_platform::Raspi4;

use crate::KERNEL;

const TASK_COUNT: usize = 3;

pub(crate) struct CooperativeYield;

impl CooperativeYield {
    pub(crate) const fn command() -> Command<Raspi4> {
        Command::new("demo", Self::run)
    }

    extern "C" fn task(id: TaskId) -> ! {
        let mut logger = KERNEL.logger();
        let priority = Priority::new(id.get());
        logger
            .debug(format_args!(
                "task {id} (priority {}): before yield",
                priority.get()
            ))
            .ok();
        CurrentTask::yield_now();
        logger
            .debug(format_args!(
                "task {id} (priority {}): after yield",
                priority.get()
            ))
            .ok();
        CurrentTask::exit();
    }

    fn run(kernel: &Kernel<Raspi4>, _arguments: &str) -> CommandResult {
        let mut logger = kernel.logger();
        logger.info("trains-rs: cooperative yield example").ok();

        for index in 0..TASK_COUNT {
            let priority = Priority::new(index as u32);
            if kernel.create_task(Self::task, priority).is_err() {
                logger
                    .error("trains-rs: cooperative yield example failed")
                    .ok();
                return CommandResult::Failure;
            }
        }

        if kernel
            .run_tasks()
            .is_ok_and(|outcome| outcome.exited_tasks() == TASK_COUNT)
        {
            logger
                .info("trains-rs: cooperative yield example complete")
                .ok();
            CommandResult::Success
        } else {
            logger
                .error("trains-rs: cooperative yield example failed")
                .ok();
            CommandResult::Failure
        }
    }
}

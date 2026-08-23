#![no_std]

use core::arch::{asm, global_asm};
use core::fmt::Write;

use trains_platform::{Platform, delay::DelayNs as _};
pub use trains_primitives::task::{Priority, TaskId};

global_asm!(include_str!("asm/boot.S"));
global_asm!(include_str!("asm/exceptions.S"));

mod context;
mod diagnostics;
mod exceptions;
mod interrupts;
pub mod runtime;
mod scheduler;
mod svc;
mod syscall;
mod task;

pub type TaskEntry = extern "C" fn(TaskId) -> !;
pub type KernelConsole<P> = <P as Platform>::Console;

pub use exceptions::ExceptionFrame;
pub use scheduler::{CreateError, RunError, RunOutcome};

/// Initialized kernel services available to the application layer.
pub struct Kernel<P: Platform> {
    platform: P,
    scheduler: &'static scheduler::Scheduler,
}

impl<P: Platform> Kernel<P> {
    /// Creates a kernel for the supplied hardware platform.
    pub const fn new(platform: P) -> Self {
        Self {
            platform,
            scheduler: &scheduler::SCHEDULER,
        }
    }

    /// Initializes the platform and kernel facilities needed by applications.
    pub fn initialize(&self) {
        self.platform.init();
        exceptions::init();

        let mut console = self.console();
        writeln!(
            console,
            "trains-rs: Raspberry Pi 4 / BCM2711 platform ready"
        )
        .ok();

        if exceptions::self_test() {
            writeln!(console, "trains-rs: exception handling ready").ok();
        } else {
            writeln!(console, "trains-rs: exception handling self-test failed").ok();
        }

        if context::self_test() {
            writeln!(console, "trains-rs: context switching ready").ok();
        } else {
            writeln!(console, "trains-rs: context switching self-test failed").ok();
        }

        if task::self_test() {
            writeln!(console, "trains-rs: task primitive ready").ok();
        } else {
            writeln!(console, "trains-rs: task primitive self-test failed").ok();
        }

        if diagnostics::cooperative_scheduler::run() {
            writeln!(console, "trains-rs: cooperative scheduling ready").ok();
        } else {
            writeln!(
                console,
                "trains-rs: cooperative scheduling self-test failed"
            )
            .ok();
        }

        let mut timer = self.platform.timer();
        timer.delay_us(1_000);
        writeln!(console, "trains-rs: Arm generic timer ready").ok();

        if interrupts::self_test(self.platform) {
            writeln!(console, "trains-rs: interrupt handling ready").ok();
        } else {
            writeln!(console, "trains-rs: interrupt handling self-test failed").ok();
        }
    }

    /// Returns a handle to the kernel console.
    pub fn console(&self) -> KernelConsole<P> {
        self.platform.console()
    }

    /// Adds an application task to the scheduler's ready queue.
    pub fn create_task(&self, entry: TaskEntry, priority: Priority) -> Result<TaskId, CreateError> {
        self.scheduler.create(entry, priority)
    }

    /// Runs all ready application tasks until they have exited.
    pub fn run_tasks(&self) -> Result<RunOutcome, RunError> {
        self.scheduler.run()
    }

    /// Dispatches an exception delivered by the architecture entry stub.
    #[doc(hidden)]
    pub fn handle_exception(&self, frame: &mut ExceptionFrame) {
        exceptions::handle(self, frame);
    }

    /// Enters the kernel's terminal idle state.
    pub fn park(&self) -> ! {
        loop {
            // SAFETY: this is the terminal idle path.
            unsafe { asm!("wfe", options(nomem, nostack, preserves_flags)) };
        }
    }
}

/// Operations available to the task currently executing in user mode.
pub struct CurrentTask;

impl CurrentTask {
    /// Places the current task at the back of its priority level.
    pub fn yield_now() {
        syscall::yield_now();
    }

    /// Permanently exits the current task.
    pub fn exit() -> ! {
        syscall::exit()
    }
}

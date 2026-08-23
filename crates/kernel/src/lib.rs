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
mod task;

pub type TaskEntry = extern "C" fn(TaskId) -> !;
pub type KernelConsole<P> = <P as Platform>::Console;

pub use exceptions::ExceptionFrame;
pub use scheduler::{CreateError, RunError, RunOutcome};

/// Initialized kernel services available to the application layer.
pub struct Kernel<P: Platform> {
    platform: P,
    context_switcher: context::ContextSwitcher,
    exception_handler: exceptions::ExceptionHandler,
    interrupt_handler: interrupts::InterruptHandler,
    scheduler_diagnostic:
        &'static diagnostics::cooperative_scheduler::CooperativeSchedulerDiagnostic,
    scheduler: &'static scheduler::Scheduler,
}

impl<P: Platform> Kernel<P> {
    /// Creates a kernel for the supplied hardware platform.
    pub const fn new(platform: P) -> Self {
        Self {
            platform,
            context_switcher: context::ContextSwitcher::new(),
            exception_handler: exceptions::ExceptionHandler::new(),
            interrupt_handler: interrupts::InterruptHandler::new(),
            scheduler_diagnostic: &diagnostics::cooperative_scheduler::COOPERATIVE_SCHEDULER,
            scheduler: &scheduler::SCHEDULER,
        }
    }

    /// Initializes the platform and kernel facilities needed by applications.
    pub fn initialize(&self) {
        self.platform.init();
        self.exception_handler.init();

        let mut console = self.console();
        writeln!(
            console,
            "trains-rs: Raspberry Pi 4 / BCM2711 platform ready"
        )
        .ok();

        if self.exception_handler.self_test() {
            writeln!(console, "trains-rs: exception handling ready").ok();
        } else {
            writeln!(console, "trains-rs: exception handling self-test failed").ok();
        }

        if self.context_switcher.self_test() {
            writeln!(console, "trains-rs: context switching ready").ok();
        } else {
            writeln!(console, "trains-rs: context switching self-test failed").ok();
        }

        if task::Task::self_test() {
            writeln!(console, "trains-rs: task primitive ready").ok();
        } else {
            writeln!(console, "trains-rs: task primitive self-test failed").ok();
        }

        if self.scheduler_diagnostic.run(self.scheduler) {
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

        if self
            .interrupt_handler
            .self_test(self.platform, &self.exception_handler)
        {
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
        match self.exception_handler.decode(frame) {
            Ok(exception) => self.handle_decoded_exception(exception, frame),
            Err(exception) => self.handle_unhandled_exception(exception, frame),
        }
    }

    fn handle_decoded_exception(
        &self,
        exception: exceptions::DecodedException,
        frame: &mut ExceptionFrame,
    ) {
        match exception {
            exceptions::DecodedException::Interrupt => self.interrupt_handler.handle(self.platform),
            exceptions::DecodedException::KernelCall(call) => self.handle_kernel_call(call, frame),
            exceptions::DecodedException::UserCall(call) => self.handle_user_call(call, frame),
        }
    }

    fn handle_kernel_call(&self, call: svc::KernelCall, frame: &mut ExceptionFrame) {
        match call {
            svc::KernelCall::ExceptionSelfTest => self.exception_handler.mark_self_test_handled(),
            svc::KernelCall::ContextSelfTest => self.context_switcher.handle_self_test(frame),
            svc::KernelCall::StartScheduler => {
                self.scheduler.start_from(&self.context_switcher, frame)
            }
        }
    }

    fn handle_user_call(&self, call: svc::UserCall, frame: &mut ExceptionFrame) {
        match call {
            svc::UserCall::Yield => self.scheduler.yield_current(&self.context_switcher, frame),
            svc::UserCall::Exit => self.scheduler.exit_current(&self.context_switcher, frame),
        }
    }

    fn handle_unhandled_exception(
        &self,
        exception: exceptions::UnhandledException,
        frame: &ExceptionFrame,
    ) -> ! {
        let mut console = self.console();
        writeln!(console, "trains-rs: unhandled exception").ok();
        writeln!(console, "  vector: {:?}", exception.vector()).ok();
        writeln!(console, "  elr:    {:#018x}", frame.elr).ok();
        writeln!(console, "  spsr:   {:#018x}", frame.spsr).ok();
        writeln!(console, "  esr:    {:#018x}", frame.esr).ok();
        writeln!(console, "  far:    {:#018x}", frame.far).ok();
        self.park()
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
        // SAFETY: the kernel installs a handler for this SVC before tasks run.
        unsafe { asm!("svc #{svc}", svc = const svc::UserCall::Yield as u16) };
    }

    /// Permanently exits the current task.
    pub fn exit() -> ! {
        // SAFETY: the kernel installs a handler for this SVC before tasks run.
        // A correctly handled exit never restores this task's context.
        unsafe { asm!("svc #{svc}", svc = const svc::UserCall::Exit as u16) };
        loop {
            core::hint::spin_loop();
        }
    }
}

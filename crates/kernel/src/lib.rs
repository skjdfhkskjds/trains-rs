#![no_std]

use core::arch::asm;
#[cfg(target_os = "none")]
use core::arch::global_asm;
use core::fmt::Write;
use core::mem::MaybeUninit;

use trains_logger::Logger;
use trains_platform::{Platform, delay::DelayNs as _};
pub use trains_primitives::task::{Priority, TaskId};

#[cfg(target_os = "none")]
global_asm!(include_str!("asm/boot.S"));
#[cfg(target_os = "none")]
global_asm!(include_str!("asm/exceptions.S"));

mod context;
mod diagnostics;
mod exceptions;
mod interrupts;
mod ipc;
pub mod runtime;
mod scheduler;
mod svc;
mod task;
mod user_memory;

/// Entry ABI for an EL0 task.
///
/// The never return type makes K1's lifecycle policy explicit: a terminating
/// task must call [`CurrentTask::exit`] rather than return from its entrypoint.
pub type TaskEntry = extern "C" fn(TaskId) -> !;
pub type KernelConsole<P> = <P as Platform>::Console;
pub type KernelLogger<P> = Logger<KernelConsole<P>>;

pub use exceptions::ExceptionFrame;
pub use ipc::IpcError;
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

        let mut logger = self.logger();
        logger
            .info("trains-rs: Raspberry Pi 4 / BCM2711 platform ready")
            .ok();

        if self.exception_handler.self_test() {
            logger.info("trains-rs: exception handling ready").ok();
        } else {
            logger
                .error("trains-rs: exception handling self-test failed")
                .ok();
        }

        if self.context_switcher.self_test() {
            logger.info("trains-rs: context switching ready").ok();
        } else {
            logger
                .error("trains-rs: context switching self-test failed")
                .ok();
        }

        if task::Task::self_test() {
            logger.info("trains-rs: task primitive ready").ok();
        } else {
            logger
                .error("trains-rs: task primitive self-test failed")
                .ok();
        }

        if self.scheduler_diagnostic.run(self.scheduler) {
            logger.info("trains-rs: cooperative scheduling ready").ok();
        } else {
            logger
                .error("trains-rs: cooperative scheduling self-test failed")
                .ok();
        }

        let mut timer = self.platform.timer();
        timer.delay_us(1_000);
        logger.info("trains-rs: Arm generic timer ready").ok();

        if self
            .interrupt_handler
            .self_test(self.platform, &self.exception_handler)
        {
            logger.info("trains-rs: interrupt handling ready").ok();
        } else {
            logger
                .error("trains-rs: interrupt handling self-test failed")
                .ok();
        }
    }

    /// Returns a handle to the kernel console.
    pub fn console(&self) -> KernelConsole<P> {
        self.platform.console()
    }

    /// Returns a logger backed by the kernel console.
    pub fn logger(&self) -> KernelLogger<P> {
        Logger::new(self.console())
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
            svc::UserCall::Create => {
                let request = svc::CreateRequest::decode(frame.registers[0], frame.registers[1]);
                self.scheduler
                    .create_current(&self.context_switcher, frame, request);
            }
            svc::UserCall::MyTid => self.scheduler.current_id(&self.context_switcher, frame),
            svc::UserCall::MyParentTid => self
                .scheduler
                .current_parent_id(&self.context_switcher, frame),
            svc::UserCall::Send => {
                let request = svc::SendRequest::decode(
                    frame.registers[0],
                    frame.registers[1],
                    frame.registers[2],
                    frame.registers[3],
                    frame.registers[4],
                );
                self.scheduler
                    .send_current(&self.context_switcher, frame, request);
            }
            svc::UserCall::Receive => {
                let request = svc::ReceiveRequest::decode(
                    frame.registers[0],
                    frame.registers[1],
                    frame.registers[2],
                );
                self.scheduler
                    .receive_current(&self.context_switcher, frame, request);
            }
            svc::UserCall::Reply => {
                let request = svc::ReplyRequest::decode(
                    frame.registers[0],
                    frame.registers[1],
                    frame.registers[2],
                );
                self.scheduler
                    .reply_current(&self.context_switcher, frame, request);
            }
        }
    }

    fn handle_unhandled_exception(
        &self,
        exception: exceptions::UnhandledException,
        frame: &ExceptionFrame,
    ) -> ! {
        let mut logger = self.logger();
        logger.error("trains-rs: unhandled exception").ok();

        let mut console = self.console();
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
    /// Creates a ready child whose parent is the calling task.
    ///
    /// Every `u32` priority is valid; lower values run first. The call is a
    /// scheduling point, so an equal- or higher-priority child can run before
    /// this function returns.
    pub fn create(priority: Priority, entry: TaskEntry) -> Result<TaskId, CreateError> {
        let result: u64;
        // SAFETY: `entry` has the kernel's task-entry ABI. The exception stub
        // preserves the suspended task context, and x0 is the defined result.
        // This intentionally has a memory clobber because another task may run.
        unsafe {
            asm!(
                "svc #{svc}",
                svc = const svc::UserCall::Create as u16,
                inout("x0") u64::from(priority.get()) => result,
                in("x1") entry as usize,
            )
        };
        svc::decode_create_result(result)
    }

    /// Returns the identity assigned to the calling task.
    ///
    /// Identity queries are scheduling points.
    pub fn id() -> TaskId {
        let result: u64;
        // SAFETY: the kernel writes the caller's task ID to the saved x0 and
        // eventually restores this context.
        unsafe {
            asm!(
                "svc #{svc}",
                svc = const svc::UserCall::MyTid as u16,
                lateout("x0") result,
            )
        };
        TaskId::new(result as u32)
    }

    /// Returns the creator of this task, or `None` for a bootstrap task.
    ///
    /// Parent identity queries are scheduling points.
    pub fn parent_id() -> Option<TaskId> {
        let result: u64;
        // SAFETY: the kernel writes the encoded optional parent to the saved
        // x0 and eventually restores this context.
        unsafe {
            asm!(
                "svc #{svc}",
                svc = const svc::UserCall::MyParentTid as u16,
                lateout("x0") result,
            )
        };
        svc::decode_parent(result)
    }

    /// Sends a byte message and blocks until its receiver replies.
    ///
    /// The returned value is the reply's logical length. At most
    /// `reply.len()` bytes are copied, so a larger result reports truncation.
    pub fn send(receiver: TaskId, message: &[u8], reply: &mut [u8]) -> Result<usize, IpcError> {
        let result: u64;
        // SAFETY: the borrowed buffers remain live and inaccessible to this
        // task while the kernel keeps it blocked. The SVC is a compiler memory
        // barrier because another task and the kernel may access the buffers.
        unsafe {
            asm!(
                "svc #{svc}",
                svc = const svc::UserCall::Send as u16,
                inlateout("x0") u64::from(receiver) => result,
                in("x1") message.as_ptr(),
                in("x2") message.len(),
                in("x3") reply.as_mut_ptr(),
                in("x4") reply.len(),
            )
        };
        svc::decode_ipc_result(result)
    }

    /// Receives the oldest waiting byte message, blocking if none is ready.
    ///
    /// The result contains the sender ID and the message's logical length. At
    /// most `message.len()` bytes are copied into the supplied buffer.
    pub fn receive(message: &mut [u8]) -> Result<(TaskId, usize), IpcError> {
        let mut sender = MaybeUninit::<TaskId>::uninit();
        let result: u64;
        // SAFETY: the kernel writes `sender` before returning a successful
        // result. Neither retained destination is exposed as a Rust reference,
        // and both remain live while this task is receive-blocked.
        unsafe {
            asm!(
                "svc #{svc}",
                svc = const svc::UserCall::Receive as u16,
                inlateout("x0") sender.as_mut_ptr() => result,
                in("x1") message.as_mut_ptr(),
                in("x2") message.len(),
            )
        };
        let length = svc::decode_ipc_result(result)?;
        // SAFETY: a successful Receive result is written only after the kernel
        // stores a complete TaskId at the supplied destination.
        Ok((unsafe { sender.assume_init() }, length))
    }

    /// Replies to a sender currently blocked on this receiving task.
    ///
    /// The returned value is the reply's logical length, even if the sender's
    /// reply buffer is smaller and receives only a prefix.
    pub fn reply(sender: TaskId, reply: &[u8]) -> Result<usize, IpcError> {
        let result: u64;
        // SAFETY: the immutable reply buffer stays live for the duration of
        // the call. The kernel copies it before making this task runnable.
        unsafe {
            asm!(
                "svc #{svc}",
                svc = const svc::UserCall::Reply as u16,
                inlateout("x0") u64::from(sender) => result,
                in("x1") reply.as_ptr(),
                in("x2") reply.len(),
            )
        };
        svc::decode_ipc_result(result)
    }

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

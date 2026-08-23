#![no_std]

use core::arch::{asm, global_asm};
use core::fmt::Write;

use trains_platform::{Platform as _, RASPI4, Raspi4, delay::DelayNs as _};
use trains_primitives::task::TaskId;

global_asm!(include_str!("boot.S"));
global_asm!(include_str!("exceptions.S"));

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

pub use scheduler::{CreateError, RunError, RunOutcome};

/// Initializes the platform and the kernel facilities needed by applications.
pub fn initialize() -> <Raspi4 as trains_platform::Platform>::Console {
    let platform = RASPI4;
    platform.init();
    exceptions::init();

    let mut console = platform.console();
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

    let mut timer = platform.timer();
    timer.delay_us(1_000);
    writeln!(console, "trains-rs: Arm generic timer ready").ok();

    if interrupts::self_test() {
        writeln!(console, "trains-rs: interrupt handling ready").ok();
    } else {
        writeln!(console, "trains-rs: interrupt handling self-test failed").ok();
    }

    console
}

/// Adds an application task to the cooperative scheduler's ready queue.
pub fn create_task(entry: TaskEntry) -> Result<TaskId, CreateError> {
    scheduler::create(entry)
}

/// Runs all ready application tasks until they have exited.
pub fn run_tasks() -> Result<RunOutcome, RunError> {
    scheduler::run()
}

/// Places the calling application task at the back of the ready queue.
pub fn yield_now() {
    syscall::yield_now();
}

/// Permanently exits the calling application task.
pub fn exit_task() -> ! {
    syscall::exit()
}

/// Enters the kernel's terminal idle state.
pub fn park() -> ! {
    loop {
        // SAFETY: this is the terminal idle path.
        unsafe { asm!("wfe", options(nomem, nostack, preserves_flags)) };
    }
}

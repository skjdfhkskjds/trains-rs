//! Safe EL0-facing wrappers around the kernel's supervisor-call ABI.

use core::arch::asm;

use crate::svc::UserCall;

/// Places the calling task at the back of the scheduler's ready queue.
pub(crate) fn yield_now() {
    // SAFETY: the kernel installs a handler for this SVC before tasks start.
    unsafe { asm!("svc #{svc}", svc = const UserCall::Yield as u16) };
}

/// Permanently removes the calling task from the current scheduler run.
pub(crate) fn exit() -> ! {
    // SAFETY: the kernel installs a handler for this SVC before tasks start.
    // A correctly handled exit never restores this task's context.
    unsafe { asm!("svc #{svc}", svc = const UserCall::Exit as u16) };
    loop {
        core::hint::spin_loop();
    }
}

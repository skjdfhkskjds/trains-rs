//! GIC interrupt dispatch and the physical-timer boot diagnostic.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

use trains_platform::{InterruptClaim as _, InterruptController as _, Platform, Timer as _};
use trains_primitives::time::Duration;

use crate::exceptions::ExceptionHandler;

/// Owns interrupt dispatch and its physical-timer diagnostic state.
pub(crate) struct InterruptHandler {
    timer_fired: AtomicBool,
}

impl InterruptHandler {
    pub(crate) const fn new() -> Self {
        Self {
            timer_fired: AtomicBool::new(false),
        }
    }

    pub(crate) fn handle<P: Platform>(&self, platform: P) {
        let controller = platform.interrupt_controller();

        while let Some(claim) = controller.claim() {
            if claim.interrupt() == Some(platform.timer_interrupt()) {
                platform.timer().cancel_deadline();
                self.timer_fired.store(true, Ordering::Release);
            }

            controller.complete(claim);
        }
    }

    pub(crate) fn self_test<P: Platform>(
        &self,
        platform: P,
        exception_handler: &ExceptionHandler,
    ) -> bool {
        let controller = platform.interrupt_controller();
        let timer = platform.timer();
        let timer_interrupt = platform.timer_interrupt();

        self.timer_fired.store(false, Ordering::Relaxed);
        timer.cancel_deadline();
        controller.enable(timer_interrupt);
        timer.schedule_after(Duration::from_millis(1));
        exception_handler.enable();

        while !self.timer_fired.load(Ordering::Acquire) {
            // SAFETY: IRQ delivery is enabled and the timer is scheduled, so
            // the processor may sleep until an interrupt becomes pending.
            unsafe { asm!("wfi", options(nomem, nostack, preserves_flags)) };
        }

        exception_handler.disable();
        controller.disable(timer_interrupt);
        timer.cancel_deadline();
        true
    }
}

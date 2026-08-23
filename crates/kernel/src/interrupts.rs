//! GIC interrupt dispatch and the physical-timer boot diagnostic.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

use trains_platform::{InterruptClaim as _, InterruptController as _, Platform, Timer as _};
use trains_primitives::time::Duration;

static TIMER_FIRED: AtomicBool = AtomicBool::new(false);

pub(crate) fn handle<P: Platform>(platform: P) {
    let controller = platform.interrupt_controller();

    while let Some(claim) = controller.claim() {
        if claim.interrupt() == Some(platform.timer_interrupt()) {
            platform.timer().cancel_deadline();
            TIMER_FIRED.store(true, Ordering::Release);
        }

        controller.complete(claim);
    }
}

pub(crate) fn self_test<P: Platform>(platform: P) -> bool {
    let controller = platform.interrupt_controller();
    let timer = platform.timer();
    let timer_interrupt = platform.timer_interrupt();

    TIMER_FIRED.store(false, Ordering::Relaxed);
    timer.cancel_deadline();
    controller.enable(timer_interrupt);
    timer.schedule_after(Duration::from_millis(1));
    crate::exceptions::enable_irqs();

    while !TIMER_FIRED.load(Ordering::Acquire) {
        // SAFETY: IRQ delivery is enabled and the timer is scheduled, so the
        // processor may sleep until an interrupt becomes pending.
        unsafe { asm!("wfi", options(nomem, nostack, preserves_flags)) };
    }

    crate::exceptions::disable_irqs();
    controller.disable(timer_interrupt);
    timer.cancel_deadline();
    true
}

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

use trains_platform::{
    InterruptClaim as _, InterruptController as _, Platform as _, RASPI4, Timer as _,
    raspi4::Interrupt,
};
use trains_primitives::time::Duration;

static TIMER_FIRED: AtomicBool = AtomicBool::new(false);

pub fn handle() {
    let controller = RASPI4.interrupt_controller();

    while let Some(claim) = controller.claim() {
        if claim.interrupt() == Some(Interrupt::PhysicalTimer) {
            RASPI4.timer().cancel_deadline();
            TIMER_FIRED.store(true, Ordering::Release);
        }

        controller.complete(claim);
    }
}

pub fn self_test() -> bool {
    let controller = RASPI4.interrupt_controller();
    let timer = RASPI4.timer();

    TIMER_FIRED.store(false, Ordering::Relaxed);
    timer.cancel_deadline();
    controller.enable(Interrupt::PhysicalTimer);
    timer.schedule_after(Duration::from_millis(1));
    crate::exceptions::enable_irqs();

    while !TIMER_FIRED.load(Ordering::Acquire) {
        // SAFETY: IRQ delivery is enabled and the timer is scheduled, so the
        // processor may sleep until an interrupt becomes pending.
        unsafe { asm!("wfi", options(nomem, nostack, preserves_flags)) };
    }

    crate::exceptions::disable_irqs();
    controller.disable(Interrupt::PhysicalTimer);
    timer.cancel_deadline();
    true
}

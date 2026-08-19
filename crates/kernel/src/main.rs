#![no_main]
#![no_std]

use core::arch::{asm, global_asm};
use core::fmt::Write;
use core::panic::PanicInfo;
use trains_platform::{
    InterruptController as _, Platform as _, RASPI4, Timer as _, delay::DelayNs as _,
    raspi4::Interrupt,
};
use trains_primitives::time::Duration;

global_asm!(include_str!("boot.S"));

#[unsafe(no_mangle)]
pub extern "C" fn kernel_main(_dtb: usize) -> ! {
    let platform = RASPI4;
    platform.init();

    let mut console = platform.console();
    writeln!(
        console,
        "trains-rs: Raspberry Pi 4 / BCM2711 platform ready"
    )
    .ok();

    let mut timer = platform.timer();
    timer.delay_us(1_000);
    writeln!(console, "trains-rs: Arm generic timer ready").ok();

    // Poll a real interrupt source while CPU IRQ delivery remains masked. This
    // verifies the controller wiring without requiring exception vectors yet.
    let interrupts = platform.interrupt_controller();
    timer.cancel_deadline();
    interrupts.enable(Interrupt::PhysicalTimer);
    timer.schedule_after(Duration::from_millis(1));
    timer.delay_us(2_000);

    if interrupts.is_pending(Interrupt::PhysicalTimer) {
        writeln!(console, "trains-rs: interrupt controller ready").ok();
    } else {
        writeln!(console, "trains-rs: interrupt controller self-test failed").ok();
    }

    interrupts.disable(Interrupt::PhysicalTimer);
    timer.cancel_deadline();

    park()
}

fn park() -> ! {
    loop {
        // SAFETY: interrupts are disabled and this is the terminal idle path.
        unsafe { asm!("wfe", options(nomem, nostack, preserves_flags)) };
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    park()
}

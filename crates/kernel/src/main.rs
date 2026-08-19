#![no_main]
#![no_std]

use core::arch::{asm, global_asm};
use core::fmt::Write;
use core::panic::PanicInfo;
use trains_platform::{
    CompareChannel, Interrupt, InterruptController as _, Platform as _, RASPI2B, Timer as _,
};

global_asm!(include_str!("boot.S"));

#[unsafe(no_mangle)]
pub extern "C" fn kernel_main(_r0: usize, _machine_id: usize, _dtb: usize) -> ! {
    let platform = RASPI2B;
    platform.init();

    let mut console = platform.console();
    writeln!(console, "trains-rs: Raspberry Pi 2 platform ready").ok();

    let timer = platform.timer();
    timer.delay_micros(1_000);
    writeln!(console, "trains-rs: system timer ready").ok();

    // Poll a real interrupt source while CPU IRQ delivery remains masked. This
    // verifies the controller wiring without requiring exception vectors yet.
    let interrupts = platform.interrupt_controller();
    timer.clear_match(CompareChannel::One);
    interrupts.enable(Interrupt::SystemTimer1);
    timer.schedule_after(CompareChannel::One, 1_000);
    timer.delay_micros(2_000);

    if interrupts.is_pending(Interrupt::SystemTimer1) {
        writeln!(console, "trains-rs: interrupt controller ready").ok();
    } else {
        writeln!(console, "trains-rs: interrupt controller self-test failed").ok();
    }

    interrupts.disable(Interrupt::SystemTimer1);
    timer.clear_match(CompareChannel::One);

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

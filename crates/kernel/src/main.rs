#![no_main]
#![no_std]

use core::arch::{asm, global_asm};
use core::fmt::Write;
use core::panic::PanicInfo;
use trains_platform::{Platform as _, RASPI4, delay::DelayNs as _};

global_asm!(include_str!("boot.S"));
global_asm!(include_str!("exceptions.S"));

mod context;
mod demo;
mod diagnostics;
mod exceptions;
mod interrupts;
mod scheduler;
mod svc;
mod syscall;
mod task;

#[unsafe(no_mangle)]
pub extern "C" fn kernel_main(_dtb: usize) -> ! {
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

    writeln!(console, "trains-rs: cooperative yield example").ok();
    if demo::cooperative_yield::run() {
        writeln!(console, "trains-rs: cooperative yield example complete").ok();
    } else {
        writeln!(console, "trains-rs: cooperative yield example failed").ok();
    }

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

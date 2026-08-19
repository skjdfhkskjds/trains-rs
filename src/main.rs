#![no_main]
#![no_std]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::ptr::{read_volatile, write_volatile};

global_asm!(include_str!("boot.S"));

const UART0_BASE: usize = 0x3f20_1000;
const UART_DR: usize = 0x00;
const UART_FR: usize = 0x18;
const UART_IBRD: usize = 0x24;
const UART_FBRD: usize = 0x28;
const UART_LCRH: usize = 0x2c;
const UART_CR: usize = 0x30;
const UART_ICR: usize = 0x44;

const UART_FR_TXFF: u32 = 1 << 5;

#[unsafe(no_mangle)]
pub extern "C" fn kernel_main(_r0: usize, _machine_id: usize, _dtb: usize) -> ! {
    uart_init();
    uart_write(b"trains-rs: entered Rust kernel\n");
    park()
}

fn uart_init() {
    // SAFETY: these are the BCM2836 PL011 registers. Boot is single-core here.
    unsafe {
        uart_write_reg(UART_CR, 0);
        uart_write_reg(UART_ICR, 0x7ff);

        // 48 MHz UART clock, 115200 baud: divisor 26 + 3/64.
        uart_write_reg(UART_IBRD, 26);
        uart_write_reg(UART_FBRD, 3);
        uart_write_reg(UART_LCRH, (3 << 5) | (1 << 4));
        uart_write_reg(UART_CR, (1 << 9) | (1 << 8) | 1);
    }
}

fn uart_write(bytes: &[u8]) {
    for &byte in bytes {
        if byte == b'\n' {
            uart_putc(b'\r');
        }
        uart_putc(byte);
    }
}

fn uart_putc(byte: u8) {
    // SAFETY: volatile accesses are required for the memory-mapped PL011.
    unsafe {
        while read_volatile((UART0_BASE + UART_FR) as *const u32) & UART_FR_TXFF != 0 {}
        uart_write_reg(UART_DR, u32::from(byte));
    }
}

unsafe fn uart_write_reg(offset: usize, value: u32) {
    // SAFETY: callers provide a valid PL011 register offset.
    unsafe { write_volatile((UART0_BASE + offset) as *mut u32, value) };
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

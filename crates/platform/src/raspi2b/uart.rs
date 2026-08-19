use core::fmt;

use crate::mmio::{device_barrier, read32, write32};
use crate::{Console, Gpio, PinFunction, Pull};

use super::PERIPHERAL_BASE;
use super::gpio::Bcm2836Gpio;

const UART0_BASE: usize = PERIPHERAL_BASE + 0x20_1000;

const DR: usize = 0x00;
const FR: usize = 0x18;
const IBRD: usize = 0x24;
const FBRD: usize = 0x28;
const LCRH: usize = 0x2c;
const CR: usize = 0x30;
const IMSC: usize = 0x38;
const MIS: usize = 0x40;
const ICR: usize = 0x44;

const FR_RXFE: u32 = 1 << 4;
const FR_TXFF: u32 = 1 << 5;
const LCRH_FEN: u32 = 1 << 4;
const LCRH_WLEN_8: u32 = 0b11 << 5;
const CR_UARTEN: u32 = 1;
const CR_TXE: u32 = 1 << 8;
const CR_RXE: u32 = 1 << 9;

const ALL_INTERRUPTS: u32 = 0x7ff;

#[derive(Clone, Copy)]
pub struct Pl011 {
    base: usize,
}

pub(super) const UART0: Pl011 = Pl011 { base: UART0_BASE };

impl Pl011 {
    pub(super) fn init(&self, gpio: &Bcm2836Gpio) {
        gpio.set_function(14, PinFunction::Alt0);
        gpio.set_function(15, PinFunction::Alt0);
        gpio.set_pull(14, Pull::None);
        gpio.set_pull(15, Pull::None);

        self.write_register(CR, 0);
        self.write_register(ICR, ALL_INTERRUPTS);

        // QEMU and Raspberry Pi firmware expose a 48 MHz PL011 clock.
        // 48 MHz / (16 * 115200) = 26 + 3/64 after rounding.
        self.write_register(IBRD, 26);
        self.write_register(FBRD, 3);
        self.write_register(LCRH, LCRH_WLEN_8 | LCRH_FEN);
        self.write_register(IMSC, 0);
        self.write_register(CR, CR_UARTEN | CR_TXE | CR_RXE);
        device_barrier();
    }

    #[inline]
    fn read_register(&self, offset: usize) -> u32 {
        // SAFETY: every call uses a PL011 register offset.
        unsafe { read32(self.base + offset) }
    }

    #[inline]
    fn write_register(&self, offset: usize, value: u32) {
        // SAFETY: every call uses a PL011 register offset.
        unsafe { write32(self.base + offset, value) };
    }
}

impl Console for Pl011 {
    fn write_byte(&self, byte: u8) {
        while self.read_register(FR) & FR_TXFF != 0 {}
        self.write_register(DR, u32::from(byte));
    }

    fn try_write_byte(&self, byte: u8) -> bool {
        if self.read_register(FR) & FR_TXFF != 0 {
            return false;
        }
        self.write_register(DR, u32::from(byte));
        true
    }

    fn read_byte(&self) -> u8 {
        while self.read_register(FR) & FR_RXFE != 0 {}
        self.read_register(DR) as u8
    }

    fn try_read_byte(&self) -> Option<u8> {
        if self.read_register(FR) & FR_RXFE != 0 {
            None
        } else {
            Some(self.read_register(DR) as u8)
        }
    }

    fn set_interrupt_mask(&self, mask: u32) {
        self.write_register(IMSC, mask & ALL_INTERRUPTS);
    }

    fn masked_interrupt_status(&self) -> u32 {
        self.read_register(MIS)
    }

    fn clear_interrupts(&self, mask: u32) {
        self.write_register(ICR, mask & ALL_INTERRUPTS);
    }
}

impl fmt::Write for Pl011 {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        for byte in value.bytes() {
            if byte == b'\n' {
                Console::write_byte(self, b'\r');
            }
            Console::write_byte(self, byte);
        }
        Ok(())
    }
}

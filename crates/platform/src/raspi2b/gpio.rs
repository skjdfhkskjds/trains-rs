use core::arch::asm;

use crate::mmio::{device_barrier, read32, write32};
use crate::{Gpio, PinFunction, Pull};

use super::PERIPHERAL_BASE;

const GPIO_BASE: usize = PERIPHERAL_BASE + 0x20_0000;
const GPFSEL0: usize = 0x00;
const GPSET0: usize = 0x1c;
const GPCLR0: usize = 0x28;
const GPLEV0: usize = 0x34;
const GPPUD: usize = 0x94;
const GPPUDCLK0: usize = 0x98;

const PIN_COUNT: u8 = 54;

#[derive(Clone, Copy)]
pub struct Bcm2836Gpio {
    base: usize,
}

pub(super) const GPIO: Bcm2836Gpio = Bcm2836Gpio { base: GPIO_BASE };

impl Gpio for Bcm2836Gpio {
    fn set_function(&self, pin: u8, function: PinFunction) {
        self.assert_pin(pin);

        let register = self.base + GPFSEL0 + usize::from(pin / 10) * 4;
        let shift = u32::from((pin % 10) * 3);

        // SAFETY: the validated pin selects one of the six GPFSEL registers.
        let mut value = unsafe { read32(register) };
        value &= !(0b111 << shift);
        value |= (function as u32) << shift;
        // SAFETY: `register` is the same validated GPFSEL register.
        unsafe { write32(register, value) };
    }

    fn set_pull(&self, pin: u8, pull: Pull) {
        self.assert_pin(pin);

        // The BCM2835 pull-control sequence requires 150 peripheral clock
        // cycles on either side of asserting the pin's clock bit.
        // SAFETY: all addresses are fixed BCM2836 GPIO registers.
        unsafe { write32(self.base + GPPUD, pull as u32) };
        delay_cycles();

        let clock = self.base + GPPUDCLK0 + usize::from(pin / 32) * 4;
        // SAFETY: the validated pin selects GPPUDCLK0 or GPPUDCLK1.
        unsafe { write32(clock, 1 << (pin % 32)) };
        delay_cycles();

        // SAFETY: clear the pull-control sequence registers.
        unsafe {
            write32(self.base + GPPUD, 0);
            write32(clock, 0);
        }
        device_barrier();
    }

    fn write(&self, pin: u8, high: bool) {
        self.assert_pin(pin);
        let offset = if high { GPSET0 } else { GPCLR0 };
        let register = self.base + offset + usize::from(pin / 32) * 4;

        // SAFETY: the validated pin selects a GPIO set or clear register.
        unsafe { write32(register, 1 << (pin % 32)) };
    }

    fn read(&self, pin: u8) -> bool {
        self.assert_pin(pin);
        let register = self.base + GPLEV0 + usize::from(pin / 32) * 4;

        // SAFETY: the validated pin selects a GPIO level register.
        unsafe { read32(register) & (1 << (pin % 32)) != 0 }
    }
}

impl Bcm2836Gpio {
    fn assert_pin(&self, pin: u8) {
        assert!(pin < PIN_COUNT, "invalid Raspberry Pi GPIO pin");
    }
}

#[inline(never)]
fn delay_cycles() {
    for _ in 0..150 {
        // SAFETY: a NOP has no side effects beyond consuming a CPU cycle.
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}

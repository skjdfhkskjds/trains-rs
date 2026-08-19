use crate::mmio::{device_barrier, read32, write32};
use crate::{Gpio, PinFunction, Pull};

use super::PERIPHERAL_BASE;

const GPIO_BASE: usize = PERIPHERAL_BASE + 0x20_0000;
const GPFSEL0: usize = 0x00;
const GPSET0: usize = 0x1c;
const GPCLR0: usize = 0x28;
const GPLEV0: usize = 0x34;
const GPIO_PUP_PDN_CNTRL_REG0: usize = 0xe4;

const PIN_COUNT: u8 = 54;

#[derive(Clone, Copy)]
pub struct Bcm2711Gpio {
    base: usize,
}

pub(super) const GPIO: Bcm2711Gpio = Bcm2711Gpio { base: GPIO_BASE };

impl Gpio for Bcm2711Gpio {
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

        let register = self.base + GPIO_PUP_PDN_CNTRL_REG0 + usize::from(pin / 16) * 4;
        let shift = u32::from((pin % 16) * 2);
        // BCM2711 uses a direct two-bit pull field per pin: 0=none, 1=up,
        // 2=down. The public enum retains the older BCM2835 ordering.
        // SAFETY: the validated pin selects one of four pull-control registers.
        let mut value = unsafe { read32(register) };
        value &= !(0b11 << shift);
        let encoding = match pull {
            Pull::None => 0,
            Pull::Down => 2,
            Pull::Up => 1,
        };
        value |= encoding << shift;
        // SAFETY: `register` is the same validated pull-control register.
        unsafe { write32(register, value) };
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

impl Bcm2711Gpio {
    fn assert_pin(&self, pin: u8) {
        assert!(pin < PIN_COUNT, "invalid Raspberry Pi GPIO pin");
    }
}

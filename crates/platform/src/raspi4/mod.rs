//! Raspberry Pi 4 / Compute Module 4 (BCM2711) platform implementation.

mod gpio;
mod interrupt;
mod timer;
mod uart;

pub use gpio::Bcm2711Gpio;
pub use interrupt::Gic400;
pub use timer::ArmGenericTimer;
pub use uart::Pl011;

use gpio::GPIO;
use interrupt::INTERRUPTS;
use timer::SYSTEM_TIMER;
use uart::UART0;

use crate::Platform;

pub(super) const PERIPHERAL_BASE: usize = 0xfe00_0000;

#[derive(Clone, Copy, Debug, Default)]
pub struct Raspi4;

pub const RASPI4: Raspi4 = Raspi4;

impl Platform for Raspi4 {
    type Console = Pl011;
    type Gpio = Bcm2711Gpio;
    type Timer = ArmGenericTimer;
    type InterruptController = Gic400;

    fn init(&self) {
        INTERRUPTS.init();
        UART0.init(&GPIO);
    }

    #[inline]
    fn console(&self) -> Self::Console {
        UART0
    }

    #[inline]
    fn gpio(&self) -> Self::Gpio {
        GPIO
    }

    #[inline]
    fn timer(&self) -> Self::Timer {
        SYSTEM_TIMER
    }

    #[inline]
    fn interrupt_controller(&self) -> Self::InterruptController {
        INTERRUPTS
    }
}

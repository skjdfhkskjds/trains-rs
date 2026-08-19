//! Raspberry Pi 2B (BCM2836) platform implementation.

mod gpio;
mod interrupt;
mod timer;
mod uart;

pub use gpio::Bcm2836Gpio;
pub use interrupt::Bcm2835InterruptController;
pub use timer::Bcm2835SystemTimer;
pub use uart::Pl011;

use gpio::GPIO;
use interrupt::INTERRUPTS;
use timer::SYSTEM_TIMER;
use uart::UART0;

use crate::Platform;

pub(super) const PERIPHERAL_BASE: usize = 0x3f00_0000;
pub(super) const LOCAL_PERIPHERAL_BASE: usize = 0x4000_0000;

#[derive(Clone, Copy, Debug, Default)]
pub struct Raspi2b;

pub const RASPI2B: Raspi2b = Raspi2b;

impl Platform for Raspi2b {
    type Console = Pl011;
    type Gpio = Bcm2836Gpio;
    type Timer = Bcm2835SystemTimer;
    type InterruptController = Bcm2835InterruptController;

    fn init(&self) {
        INTERRUPTS.route_peripheral_irqs_to_core(0);
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

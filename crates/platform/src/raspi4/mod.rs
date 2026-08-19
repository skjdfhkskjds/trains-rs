//! Raspberry Pi 4 / Compute Module 4 (BCM2711) platform implementation.

mod gpio;
mod interrupt;
mod timer;
mod uart;

pub use gpio::Bcm2711Gpio;
pub use interrupt::Gic400;
pub use timer::ArmTimer;
pub use uart::Pl011;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PinFunction {
    Input = 0b000,
    Output = 0b001,
    Alt5 = 0b010,
    Alt4 = 0b011,
    Alt0 = 0b100,
    Alt1 = 0b101,
    Alt2 = 0b110,
    Alt3 = 0b111,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pull {
    None,
    Down,
    Up,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Interrupt {
    PhysicalTimer = 30,
    SystemTimer1 = 97,
    SystemTimer3 = 99,
    Auxiliary = 125,
    Uart = 153,
}

impl Interrupt {
    fn from_id(id: u16) -> Option<Self> {
        match id {
            30 => Some(Self::PhysicalTimer),
            97 => Some(Self::SystemTimer1),
            99 => Some(Self::SystemTimer3),
            125 => Some(Self::Auxiliary),
            153 => Some(Self::Uart),
            _ => None,
        }
    }
}

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
    type Timer = ArmTimer;
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

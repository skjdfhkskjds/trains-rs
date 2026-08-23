#![no_std]

use core::fmt;

pub mod delay;
pub mod io;
mod mmio;
pub mod raspi4;

pub use trains_primitives::time;

pub use raspi4::{RASPI4, Raspi4};

pub trait Console:
    Copy + fmt::Write + io::Read + io::Write + io::ReadReady + io::WriteReady
{
    /// Blocks until one byte is available from the console.
    fn read_byte(&mut self) -> Result<u8, <Self as io::ErrorType>::Error> {
        let mut byte = [0];
        loop {
            if self.read(&mut byte)? == 1 {
                return Ok(byte[0]);
            }
        }
    }
}

pub trait Gpio: Copy {
    type Pin: Copy;
    type Function: Copy;
    type Pull: Copy;

    fn set_function(&self, pin: Self::Pin, function: Self::Function);
    fn set_pull(&self, pin: Self::Pin, pull: Self::Pull);
    fn write(&self, pin: Self::Pin, high: bool);
    fn read(&self, pin: Self::Pin) -> bool;
}

pub trait Timer: Copy + delay::DelayNs {
    fn now(&self) -> time::Instant;
    fn schedule_after(&self, duration: time::Duration);
    fn set_deadline(&self, deadline: time::Instant);
    fn cancel_deadline(&self);
}

pub trait InterruptController: Copy {
    type Interrupt: Copy + Eq;
    type Claim: InterruptClaim<Interrupt = Self::Interrupt>;

    fn enable(&self, interrupt: Self::Interrupt);
    fn disable(&self, interrupt: Self::Interrupt);
    fn is_pending(&self, interrupt: Self::Interrupt) -> bool;
    fn claim(&self) -> Option<Self::Claim>;
    fn complete(&self, claim: Self::Claim);
}

/// Interrupt controls exposed by an interrupt-producing peripheral.
pub trait InterruptSource: Copy {
    type Mask: Copy;

    fn set_interrupt_mask(&self, mask: Self::Mask);
    fn masked_interrupt_status(&self) -> Self::Mask;
    fn clear_interrupts(&self, mask: Self::Mask);
}

pub trait InterruptClaim: Copy {
    type Interrupt: Copy;

    fn interrupt(self) -> Option<Self::Interrupt>;
}

/// Hardware capabilities required by the kernel base.
pub trait Platform: Copy {
    type Console: Console;
    type Gpio: Gpio;
    type Timer: Timer;
    type InterruptController: InterruptController;

    fn init(&self);
    fn console(&self) -> Self::Console;
    fn gpio(&self) -> Self::Gpio;
    fn timer(&self) -> Self::Timer;
    fn interrupt_controller(&self) -> Self::InterruptController;
    fn timer_interrupt(&self) -> <Self::InterruptController as InterruptController>::Interrupt;
}

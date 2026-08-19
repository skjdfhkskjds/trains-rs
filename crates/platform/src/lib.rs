#![no_std]

use core::fmt;

pub mod delay;
pub mod io;
mod mmio;
pub mod raspi4;

pub use raspi4::{RASPI4, Raspi4};

pub trait Console:
    Copy + fmt::Write + io::Read + io::Write + io::ReadReady + io::WriteReady
{
    fn set_interrupt_mask(&self, mask: u32);
    fn masked_interrupt_status(&self) -> u32;
    fn clear_interrupts(&self, mask: u32);
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
    fn now(&self) -> u64;
    fn schedule_after(&self, microseconds: u32);
    fn set_deadline(&self, timestamp: u64);
    fn cancel_deadline(&self);
}

pub trait InterruptController: Copy {
    type Interrupt: Copy;

    fn enable(&self, interrupt: Self::Interrupt);
    fn disable(&self, interrupt: Self::Interrupt);
    fn is_pending(&self, interrupt: Self::Interrupt) -> bool;
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
}

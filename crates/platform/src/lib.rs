#![no_std]

use core::fmt;

mod mmio;
pub mod raspi4;

pub use raspi4::{RASPI4, Raspi4};

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
#[repr(u32)]
pub enum Pull {
    None = 0,
    Down = 1,
    Up = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CompareChannel {
    One = 1,
    Three = 3,
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

pub trait Console: Copy + fmt::Write {
    fn write_byte(&self, byte: u8);
    fn try_write_byte(&self, byte: u8) -> bool;
    fn read_byte(&self) -> u8;
    fn try_read_byte(&self) -> Option<u8>;
    fn set_interrupt_mask(&self, mask: u32);
    fn masked_interrupt_status(&self) -> u32;
    fn clear_interrupts(&self, mask: u32);
}

pub trait Gpio: Copy {
    fn set_function(&self, pin: u8, function: PinFunction);
    fn set_pull(&self, pin: u8, pull: Pull);
    fn write(&self, pin: u8, high: bool);
    fn read(&self, pin: u8) -> bool;
}

pub trait Timer: Copy {
    fn now(&self) -> u64;
    fn delay_micros(&self, micros: u32);
    fn schedule_after(&self, compare: CompareChannel, micros: u32);
    fn set_compare(&self, compare: CompareChannel, value: u32);
    fn clear_match(&self, compare: CompareChannel);
}

pub trait InterruptController: Copy {
    fn enable(&self, interrupt: Interrupt);
    fn disable(&self, interrupt: Interrupt);
    fn is_pending(&self, interrupt: Interrupt) -> bool;
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

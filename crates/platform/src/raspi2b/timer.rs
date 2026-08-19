use crate::mmio::{read32, write32};
use crate::{CompareChannel, Timer};

use super::PERIPHERAL_BASE;

const SYSTEM_TIMER_BASE: usize = PERIPHERAL_BASE + 0x3000;
const CS: usize = 0x00;
const CLO: usize = 0x04;
const CHI: usize = 0x08;
const C0: usize = 0x0c;

#[derive(Clone, Copy)]
pub struct Bcm2835SystemTimer {
    base: usize,
}

pub(super) const SYSTEM_TIMER: Bcm2835SystemTimer = Bcm2835SystemTimer {
    base: SYSTEM_TIMER_BASE,
};

impl Timer for Bcm2835SystemTimer {
    /// Returns the one-megahertz free-running counter in microseconds.
    fn now(&self) -> u64 {
        loop {
            let high_before = self.read_register(CHI);
            let low = self.read_register(CLO);
            let high_after = self.read_register(CHI);

            if high_before == high_after {
                return (u64::from(high_before) << 32) | u64::from(low);
            }
        }
    }

    fn delay_micros(&self, micros: u32) {
        let start = self.counter_low();
        while self.counter_low().wrapping_sub(start) < micros {}
    }

    fn schedule_after(&self, compare: CompareChannel, micros: u32) {
        let deadline = self.counter_low().wrapping_add(micros);
        self.set_compare(compare, deadline);
    }

    fn set_compare(&self, compare: CompareChannel, value: u32) {
        self.write_register(C0 + usize::from(compare as u8) * 4, value);
    }

    fn clear_match(&self, compare: CompareChannel) {
        self.write_register(CS, 1 << (compare as u8));
    }
}

impl Bcm2835SystemTimer {
    fn counter_low(&self) -> u32 {
        self.read_register(CLO)
    }

    #[inline]
    fn read_register(&self, offset: usize) -> u32 {
        // SAFETY: every call uses a BCM2835 system-timer register offset.
        unsafe { read32(self.base + offset) }
    }

    #[inline]
    fn write_register(&self, offset: usize, value: u32) {
        // SAFETY: every call uses a BCM2835 system-timer register offset.
        unsafe { write32(self.base + offset, value) };
    }
}

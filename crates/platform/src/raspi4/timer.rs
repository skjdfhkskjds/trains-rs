use core::arch::asm;

use crate::Timer;
use crate::delay::DelayNs;

#[derive(Clone, Copy)]
pub struct ArmTimer;

pub(super) const SYSTEM_TIMER: ArmTimer = ArmTimer;

#[inline]
fn counter() -> u64 {
    let value: u64;
    // SAFETY: reading the physical counter has no side effects.
    unsafe {
        asm!("mrs {value}, cntpct_el0", value = out(reg) value, options(nomem, nostack, preserves_flags))
    };
    value
}

#[inline]
fn frequency() -> u64 {
    let value: u64;
    // SAFETY: CNTFRQ_EL0 is a read-only architectural frequency register.
    unsafe {
        asm!("mrs {value}, cntfrq_el0", value = out(reg) value, options(nomem, nostack, preserves_flags))
    };
    value
}

fn micros_to_ticks(micros: u32, frequency: u64) -> u64 {
    frequency.saturating_mul(u64::from(micros)) / 1_000_000
}

fn nanos_to_ticks(nanoseconds: u32, frequency: u64) -> u64 {
    frequency
        .saturating_mul(u64::from(nanoseconds))
        .div_ceil(1_000_000_000)
}

fn ticks_to_micros(ticks: u64, frequency: u64) -> u64 {
    ticks.saturating_mul(1_000_000) / frequency
}

impl DelayNs for ArmTimer {
    fn delay_ns(&mut self, nanoseconds: u32) {
        let start = counter();
        let duration = nanos_to_ticks(nanoseconds, frequency());
        while counter().wrapping_sub(start) < duration {}
    }
}

impl Timer for ArmTimer {
    fn now(&self) -> u64 {
        ticks_to_micros(counter(), frequency())
    }

    fn schedule_after(&self, micros: u32) {
        let ticks = micros_to_ticks(micros, frequency()).min(u32::MAX as u64);
        // SAFETY: CNTP_TVAL_EL0 and CNTP_CTL_EL0 control the current core's
        // non-secure physical timer. DAIF keeps delivery masked for now.
        unsafe {
            asm!("msr cntp_tval_el0, {ticks}", ticks = in(reg) ticks, options(nostack));
            asm!("msr cntp_ctl_el0, {control}", control = in(reg) 1_u64, options(nostack));
            asm!("isb", options(nostack, preserves_flags));
        }
    }

    fn set_deadline(&self, timestamp: u64) {
        let delay = timestamp
            .saturating_sub(self.now())
            .min(u64::from(u32::MAX)) as u32;
        self.schedule_after(delay);
    }

    fn cancel_deadline(&self) {
        // SAFETY: disabling CNTP stops and deasserts the current core's timer.
        unsafe {
            asm!("msr cntp_ctl_el0, {control}", control = in(reg) 0_u64, options(nostack));
            asm!("isb", options(nostack, preserves_flags));
        }
    }
}

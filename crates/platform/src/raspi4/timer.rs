use core::arch::asm;

use crate::Timer;
use crate::delay::DelayNs;
use crate::time::{Duration, Instant};

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

fn nanos_to_ticks(nanoseconds: u32, frequency: u64) -> u64 {
    frequency
        .saturating_mul(u64::from(nanoseconds))
        .div_ceil(1_000_000_000)
}

fn duration_to_ticks(duration: Duration, frequency: u64) -> u64 {
    duration
        .as_nanos()
        .saturating_mul(u128::from(frequency))
        .div_ceil(1_000_000_000)
        .min(u128::from(u64::MAX)) as u64
}

fn ticks_to_instant(ticks: u64, frequency: u64) -> Instant {
    let nanoseconds = u128::from(ticks)
        .saturating_mul(1_000_000_000)
        .checked_div(u128::from(frequency))
        .unwrap_or(0)
        .min(u128::from(u64::MAX)) as u64;
    Instant::new(nanoseconds)
}

impl DelayNs for ArmTimer {
    fn delay_ns(&mut self, nanoseconds: u32) {
        let start = counter();
        let duration = nanos_to_ticks(nanoseconds, frequency());
        while counter().wrapping_sub(start) < duration {}
    }
}

impl Timer for ArmTimer {
    fn now(&self) -> Instant {
        ticks_to_instant(counter(), frequency())
    }

    fn schedule_after(&self, duration: Duration) {
        let deadline = counter().saturating_add(duration_to_ticks(duration, frequency()));
        // SAFETY: CNTP_CVAL_EL0 and CNTP_CTL_EL0 control the current core's
        // non-secure physical timer. DAIF keeps delivery masked for now.
        unsafe {
            asm!("msr cntp_cval_el0, {deadline}", deadline = in(reg) deadline, options(nostack));
            asm!("msr cntp_ctl_el0, {control}", control = in(reg) 1_u64, options(nostack));
            asm!("isb", options(nostack, preserves_flags));
        }
    }

    fn set_deadline(&self, deadline: Instant) {
        self.schedule_after(deadline.saturating_duration_since(self.now()));
    }

    fn cancel_deadline(&self) {
        // SAFETY: disabling CNTP stops and deasserts the current core's timer.
        unsafe {
            asm!("msr cntp_ctl_el0, {control}", control = in(reg) 0_u64, options(nostack));
            asm!("isb", options(nostack, preserves_flags));
        }
    }
}

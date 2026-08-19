use core::ops::{Add, AddAssign, Sub, SubAssign};

use super::Duration;

/// An opaque point on a monotonic clock.
///
/// Like `std::time::Instant`, an instant is useful for comparison and
/// arithmetic with [`Duration`], but does not expose its underlying epoch.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Instant(u64);

impl Instant {
    #[doc(hidden)]
    pub const fn new(nanoseconds: u64) -> Self {
        Self(nanoseconds)
    }

    pub fn duration_since(self, earlier: Self) -> Duration {
        self.saturating_duration_since(earlier)
    }

    pub fn checked_duration_since(self, earlier: Self) -> Option<Duration> {
        self.0.checked_sub(earlier.0).map(Duration::from_nanos)
    }

    pub fn saturating_duration_since(self, earlier: Self) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }

    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        let nanoseconds = u64::try_from(duration.as_nanos()).ok()?;
        self.0.checked_add(nanoseconds).map(Self)
    }

    pub fn checked_sub(self, duration: Duration) -> Option<Self> {
        let nanoseconds = u64::try_from(duration.as_nanos()).ok()?;
        self.0.checked_sub(nanoseconds).map(Self)
    }
}

impl Add<Duration> for Instant {
    type Output = Self;

    fn add(self, duration: Duration) -> Self::Output {
        self.checked_add(duration)
            .expect("overflow when adding duration to instant")
    }
}

impl AddAssign<Duration> for Instant {
    fn add_assign(&mut self, duration: Duration) {
        *self = *self + duration;
    }
}

impl Sub<Duration> for Instant {
    type Output = Self;

    fn sub(self, duration: Duration) -> Self::Output {
        self.checked_sub(duration)
            .expect("overflow when subtracting duration from instant")
    }
}

impl SubAssign<Duration> for Instant {
    fn sub_assign(&mut self, duration: Duration) {
        *self = *self - duration;
    }
}

impl Sub for Instant {
    type Output = Duration;

    fn sub(self, earlier: Self) -> Self::Output {
        self.duration_since(earlier)
    }
}

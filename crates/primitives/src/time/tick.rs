/// A count of clock-defined scheduling quanta.
///
/// A tick has no universal duration. The clock producing it defines that
/// relationship (the original clock server, for example, uses 10 ms ticks).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Tick(u64);

impl Tick {
    pub const ZERO: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub const fn checked_add(self, count: u64) -> Option<Self> {
        match self.0.checked_add(count) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn saturating_add(self, count: u64) -> Self {
        Self(self.0.saturating_add(count))
    }
}

impl From<u64> for Tick {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

impl From<Tick> for u64 {
    fn from(value: Tick) -> Self {
        value.get()
    }
}

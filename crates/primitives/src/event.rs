//! Event identifiers shared by the kernel and user tasks.

/// Identifies an event that a task can await or signal.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct EventId(u32);

impl EventId {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for EventId {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<EventId> for u32 {
    fn from(value: EventId) -> Self {
        value.get()
    }
}

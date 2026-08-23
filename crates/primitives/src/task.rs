//! Task identity and scheduling types.

use core::fmt;
use core::num::TryFromIntError;

/// Identity assigned to a kernel task.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct TaskId(u32);

impl TaskId {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for TaskId {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<TaskId> for u32 {
    fn from(value: TaskId) -> Self {
        value.get()
    }
}

impl From<TaskId> for u64 {
    fn from(value: TaskId) -> Self {
        Self::from(value.get())
    }
}

impl TryFrom<usize> for TaskId {
    type Error = TryFromIntError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        u32::try_from(value).map(Self::new)
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Scheduling priority assigned to a task.
///
/// Lower numeric values represent higher scheduling priority, matching the
/// convention used by the original kernel.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Priority(u32);

impl Priority {
    pub const HIGHEST: Self = Self(u32::MIN);
    pub const LOWEST: Self = Self(u32::MAX);

    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for Priority {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<Priority> for u32 {
    fn from(value: Priority) -> Self {
        value.get()
    }
}

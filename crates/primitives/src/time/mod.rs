//! Monotonic time types for kernel and platform interfaces.

mod instant;
mod tick;

pub use core::time::Duration;
pub use instant::Instant;
pub use tick::Tick;

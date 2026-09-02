#![no_std]

use core::fmt::{self, Display};

use trains_platform::Console;

/// Severity attached to a log line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    Debug,
    Info,
    Warning,
    Error,
}

impl Level {
    const fn header(self) -> &'static str {
        match self {
            Self::Debug => "\x1b[35m[DEBUG]\x1b[0m",
            Self::Info => "\x1b[94m[INFO]\x1b[0m",
            Self::Warning => "\x1b[33m[WARN]\x1b[0m",
            Self::Error => "\x1b[31m[ERROR]\x1b[0m",
        }
    }
}

/// Writes color-coded, severity-prefixed lines to a console.
pub struct Logger<C: Console> {
    console: C,
}

impl<C: Console> Logger<C> {
    /// Wraps a console with structured line logging.
    pub const fn new(console: C) -> Self {
        Self { console }
    }

    /// Writes one line at the supplied severity.
    pub fn log(&mut self, level: Level, message: impl Display) -> fmt::Result {
        writeln!(self.console, "{} {}", level.header(), message)
    }

    /// Writes a magenta debug line.
    pub fn debug(&mut self, message: impl Display) -> fmt::Result {
        self.log(Level::Debug, message)
    }

    /// Writes a light-blue informational line.
    pub fn info(&mut self, message: impl Display) -> fmt::Result {
        self.log(Level::Info, message)
    }

    /// Writes a yellow warning line.
    pub fn warning(&mut self, message: impl Display) -> fmt::Result {
        self.log(Level::Warning, message)
    }

    /// Writes a red error line.
    pub fn error(&mut self, message: impl Display) -> fmt::Result {
        self.log(Level::Error, message)
    }

    /// Removes the logging wrapper and returns the underlying console.
    pub fn into_inner(self) -> C {
        self.console
    }
}

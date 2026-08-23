//! Typed supervisor-call numbers shared by exception dispatch and callers.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UnknownCall;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub(crate) enum UserCall {
    Yield = 1,
    Exit = 2,
}

impl TryFrom<u16> for UserCall {
    type Error = UnknownCall;

    fn try_from(number: u16) -> Result<Self, Self::Error> {
        match number {
            value if value == Self::Yield as u16 => Ok(Self::Yield),
            value if value == Self::Exit as u16 => Ok(Self::Exit),
            _ => Err(UnknownCall),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub(crate) enum KernelCall {
    ExceptionSelfTest = 0x54,
    ContextSelfTest = 0x55,
    StartScheduler = 0x56,
}

impl TryFrom<u16> for KernelCall {
    type Error = UnknownCall;

    fn try_from(number: u16) -> Result<Self, Self::Error> {
        match number {
            value if value == Self::ExceptionSelfTest as u16 => Ok(Self::ExceptionSelfTest),
            value if value == Self::ContextSelfTest as u16 => Ok(Self::ContextSelfTest),
            value if value == Self::StartScheduler as u16 => Ok(Self::StartScheduler),
            _ => Err(UnknownCall),
        }
    }
}

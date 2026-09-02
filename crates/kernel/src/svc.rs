//! Typed supervisor-call numbers and the raw K1 ABI boundary.

use trains_primitives::task::{Priority, TaskId};

use crate::context::EntryPoint;
use crate::scheduler::CreateError;

const NO_PARENT: u64 = u64::MAX;
const CREATE_INVALID_PRIORITY: u64 = u64::MAX;
const CREATE_CAPACITY_REACHED: u64 = u64::MAX - 1;
const CREATE_TASK_ID_UNAVAILABLE: u64 = u64::MAX - 2;
const CREATE_SCHEDULER_RUNNING: u64 = u64::MAX - 3;
const CREATE_INVALID_ENTRY_POINT: u64 = u64::MAX - 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UnknownCall;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub(crate) enum UserCall {
    Yield = 1,
    Exit = 2,
    Create = 3,
    MyTid = 4,
    MyParentTid = 5,
}

impl TryFrom<u16> for UserCall {
    type Error = UnknownCall;

    fn try_from(number: u16) -> Result<Self, Self::Error> {
        match number {
            value if value == Self::Yield as u16 => Ok(Self::Yield),
            value if value == Self::Exit as u16 => Ok(Self::Exit),
            value if value == Self::Create as u16 => Ok(Self::Create),
            value if value == Self::MyTid as u16 => Ok(Self::MyTid),
            value if value == Self::MyParentTid as u16 => Ok(Self::MyParentTid),
            _ => Err(UnknownCall),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CreateRequest {
    pub(crate) entry: EntryPoint,
    pub(crate) priority: Priority,
}

impl CreateRequest {
    /// Decodes the only raw values accepted by the K1 `Create` boundary.
    ///
    /// The safe caller always supplies a valid function pointer. Without an
    /// MMU the kernel cannot prove that an arbitrary EL0 address is executable,
    /// but it still rejects null, misaligned, and non-native addresses here.
    pub(crate) fn decode(raw_priority: u64, raw_entry: u64) -> Result<Self, CreateError> {
        let priority = u32::try_from(raw_priority)
            .map(Priority::new)
            .map_err(|_| CreateError::InvalidPriority)?;
        let entry_address =
            usize::try_from(raw_entry).map_err(|_| CreateError::InvalidEntryPoint)?;
        if entry_address == 0 || entry_address & 0b11 != 0 {
            return Err(CreateError::InvalidEntryPoint);
        }

        // Keep the raw address as data instead of forging a Rust function
        // pointer. A bad executable address will take a normal EL0 exception
        // when restored; K1 otherwise trusts tasks because it has no MMU.
        let entry = EntryPoint::new(entry_address);
        Ok(Self { entry, priority })
    }
}

pub(crate) const fn encode_create_result(result: Result<TaskId, CreateError>) -> u64 {
    match result {
        Ok(id) => id.get() as u64,
        Err(CreateError::InvalidPriority) => CREATE_INVALID_PRIORITY,
        Err(CreateError::CapacityReached) => CREATE_CAPACITY_REACHED,
        Err(CreateError::TaskIdUnavailable) => CREATE_TASK_ID_UNAVAILABLE,
        Err(CreateError::SchedulerRunning) => CREATE_SCHEDULER_RUNNING,
        Err(CreateError::InvalidEntryPoint) => CREATE_INVALID_ENTRY_POINT,
    }
}

pub(crate) fn decode_create_result(raw: u64) -> Result<TaskId, CreateError> {
    match raw {
        CREATE_INVALID_PRIORITY => Err(CreateError::InvalidPriority),
        CREATE_CAPACITY_REACHED => Err(CreateError::CapacityReached),
        CREATE_TASK_ID_UNAVAILABLE => Err(CreateError::TaskIdUnavailable),
        CREATE_SCHEDULER_RUNNING => Err(CreateError::SchedulerRunning),
        CREATE_INVALID_ENTRY_POINT => Err(CreateError::InvalidEntryPoint),
        value => u32::try_from(value)
            .map(TaskId::new)
            .map_err(|_| CreateError::TaskIdUnavailable),
    }
}

pub(crate) const fn encode_parent(parent: Option<TaskId>) -> u64 {
    match parent {
        Some(id) => id.get() as u64,
        None => NO_PARENT,
    }
}

pub(crate) fn decode_parent(raw: u64) -> Option<TaskId> {
    if raw == NO_PARENT {
        None
    } else {
        u32::try_from(raw).ok().map(TaskId::new)
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

#[cfg(test)]
mod tests {
    use super::*;

    extern "C" fn task_entry(_id: TaskId) -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[test]
    fn create_request_checks_the_raw_priority_domain() {
        let entry = task_entry as *const () as usize as u64;
        let request = CreateRequest::decode(u32::MAX as u64, entry).unwrap();
        assert_eq!(request.priority, Priority::LOWEST);
        assert_eq!(
            request.entry,
            EntryPoint::new(task_entry as *const () as usize)
        );
        assert_eq!(
            CreateRequest::decode(u32::MAX as u64 + 1, entry).err(),
            Some(CreateError::InvalidPriority)
        );
    }

    #[test]
    fn create_request_rejects_malformed_entry_points() {
        assert_eq!(
            CreateRequest::decode(0, 0).err(),
            Some(CreateError::InvalidEntryPoint)
        );
        assert_eq!(
            CreateRequest::decode(0, 3).err(),
            Some(CreateError::InvalidEntryPoint)
        );
    }

    #[test]
    fn create_results_have_one_central_typed_encoding() {
        let id = TaskId::new(9);
        assert_eq!(decode_create_result(encode_create_result(Ok(id))), Ok(id));

        for error in [
            CreateError::InvalidPriority,
            CreateError::CapacityReached,
            CreateError::TaskIdUnavailable,
            CreateError::SchedulerRunning,
            CreateError::InvalidEntryPoint,
        ] {
            assert_eq!(
                decode_create_result(encode_create_result(Err(error))),
                Err(error)
            );
        }
    }

    #[test]
    fn parent_encoding_reserves_a_distinct_root_value() {
        let parent = TaskId::new(4);
        assert_eq!(decode_parent(encode_parent(Some(parent))), Some(parent));
        assert_eq!(decode_parent(encode_parent(None)), None);
    }
}

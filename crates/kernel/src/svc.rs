//! Typed supervisor-call numbers and the raw K1/K2 ABI boundary.

use trains_primitives::task::{Priority, TaskId};

use crate::context::EntryPoint;
use crate::ipc::{IpcError, OutboundMessage, ReceiveDestination};
use crate::scheduler::CreateError;
use crate::user_memory::{ReadBuffer, TaskIdDestination, WriteBuffer};

const NO_PARENT: u64 = u64::MAX;
const CREATE_INVALID_PRIORITY: u64 = u64::MAX;
const CREATE_CAPACITY_REACHED: u64 = u64::MAX - 1;
const CREATE_TASK_ID_UNAVAILABLE: u64 = u64::MAX - 2;
const CREATE_SCHEDULER_RUNNING: u64 = u64::MAX - 3;
const CREATE_INVALID_ENTRY_POINT: u64 = u64::MAX - 4;
const IPC_TASK_NOT_FOUND: u64 = u64::MAX - 16;
const IPC_NOT_REPLY_BLOCKED: u64 = u64::MAX - 17;
const IPC_NOT_MESSAGE_RECEIVER: u64 = u64::MAX - 18;
const IPC_QUEUE_FULL: u64 = u64::MAX - 19;
const IPC_INVALID_BUFFER: u64 = u64::MAX - 20;
const IPC_WOULD_DEADLOCK: u64 = u64::MAX - 21;

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
    Send = 11,
    Receive = 12,
    Reply = 13,
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
            value if value == Self::Send as u16 => Ok(Self::Send),
            value if value == Self::Receive as u16 => Ok(Self::Receive),
            value if value == Self::Reply as u16 => Ok(Self::Reply),
            _ => Err(UnknownCall),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SendRequest {
    pub(crate) receiver: TaskId,
    pub(crate) message: ReadBuffer,
    pub(crate) reply: WriteBuffer,
}

impl SendRequest {
    pub(crate) fn decode(
        raw_receiver: u64,
        raw_message: u64,
        raw_message_length: u64,
        raw_reply: u64,
        raw_reply_capacity: u64,
    ) -> Result<Self, IpcError> {
        let receiver = decode_task_id(raw_receiver)?;
        let message = ReadBuffer::decode(raw_message, raw_message_length)
            .map_err(|_| IpcError::InvalidBuffer)?;
        let reply = WriteBuffer::decode(raw_reply, raw_reply_capacity)
            .map_err(|_| IpcError::InvalidBuffer)?;
        Ok(Self {
            receiver,
            message,
            reply,
        })
    }

    pub(crate) const fn outbound(self) -> OutboundMessage {
        OutboundMessage::new(self.receiver, self.message, self.reply)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReceiveRequest {
    pub(crate) sender: TaskIdDestination,
    pub(crate) message: WriteBuffer,
}

impl ReceiveRequest {
    pub(crate) fn decode(
        raw_sender: u64,
        raw_message: u64,
        raw_message_capacity: u64,
    ) -> Result<Self, IpcError> {
        let sender = TaskIdDestination::decode(raw_sender).map_err(|_| IpcError::InvalidBuffer)?;
        let message = WriteBuffer::decode(raw_message, raw_message_capacity)
            .map_err(|_| IpcError::InvalidBuffer)?;
        Ok(Self { sender, message })
    }

    pub(crate) const fn destination(self) -> ReceiveDestination {
        ReceiveDestination::new(self.sender, self.message)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReplyRequest {
    pub(crate) sender: TaskId,
    pub(crate) reply: ReadBuffer,
}

impl ReplyRequest {
    pub(crate) fn decode(
        raw_sender: u64,
        raw_reply: u64,
        raw_reply_length: u64,
    ) -> Result<Self, IpcError> {
        let sender = decode_task_id(raw_sender)?;
        let reply =
            ReadBuffer::decode(raw_reply, raw_reply_length).map_err(|_| IpcError::InvalidBuffer)?;
        Ok(Self { sender, reply })
    }
}

fn decode_task_id(raw: u64) -> Result<TaskId, IpcError> {
    u32::try_from(raw)
        .map(TaskId::new)
        .map_err(|_| IpcError::TaskNotFound)
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

pub(crate) const fn encode_ipc_result(result: Result<usize, IpcError>) -> u64 {
    match result {
        Ok(length) if length <= isize::MAX as usize => length as u64,
        Ok(_) => IPC_INVALID_BUFFER,
        Err(IpcError::TaskNotFound) => IPC_TASK_NOT_FOUND,
        Err(IpcError::NotReplyBlocked) => IPC_NOT_REPLY_BLOCKED,
        Err(IpcError::NotMessageReceiver) => IPC_NOT_MESSAGE_RECEIVER,
        Err(IpcError::QueueFull) => IPC_QUEUE_FULL,
        Err(IpcError::InvalidBuffer) => IPC_INVALID_BUFFER,
        Err(IpcError::WouldDeadlock) => IPC_WOULD_DEADLOCK,
    }
}

pub(crate) fn decode_ipc_result(raw: u64) -> Result<usize, IpcError> {
    match raw {
        IPC_TASK_NOT_FOUND => Err(IpcError::TaskNotFound),
        IPC_NOT_REPLY_BLOCKED => Err(IpcError::NotReplyBlocked),
        IPC_NOT_MESSAGE_RECEIVER => Err(IpcError::NotMessageReceiver),
        IPC_QUEUE_FULL => Err(IpcError::QueueFull),
        IPC_INVALID_BUFFER => Err(IpcError::InvalidBuffer),
        IPC_WOULD_DEADLOCK => Err(IpcError::WouldDeadlock),
        value if value <= isize::MAX as u64 => {
            usize::try_from(value).map_err(|_| IpcError::InvalidBuffer)
        }
        _ => Err(IpcError::InvalidBuffer),
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

    #[test]
    fn ipc_requests_enforce_pointer_and_length_rules() {
        assert_eq!(
            SendRequest::decode(0, 0, 1, 0, 0),
            Err(IpcError::InvalidBuffer)
        );
        assert!(SendRequest::decode(0, 0, 0, 0, 0).is_ok());
        assert_eq!(
            ReceiveRequest::decode(0, 0, 0),
            Err(IpcError::InvalidBuffer)
        );
        assert_eq!(
            ReplyRequest::decode(u32::MAX as u64 + 1, 0, 0),
            Err(IpcError::TaskNotFound)
        );
    }

    #[test]
    fn ipc_results_have_one_central_typed_encoding() {
        assert_eq!(decode_ipc_result(encode_ipc_result(Ok(42))), Ok(42));
        assert_eq!(
            decode_ipc_result(encode_ipc_result(Ok(usize::MAX))),
            Err(IpcError::InvalidBuffer)
        );
        for error in [
            IpcError::TaskNotFound,
            IpcError::NotReplyBlocked,
            IpcError::NotMessageReceiver,
            IpcError::QueueFull,
            IpcError::InvalidBuffer,
            IpcError::WouldDeadlock,
        ] {
            assert_eq!(decode_ipc_result(encode_ipc_result(Err(error))), Err(error));
        }
    }
}

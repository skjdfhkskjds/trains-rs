//! K2 synchronous message-passing metadata and fixed-capacity queues.

use trains_primitives::task::TaskId;

use crate::user_memory::{ReadBuffer, TaskIdDestination, WriteBuffer};

/// Maximum number of senders that may wait on one receiver.
///
/// This preserves the K2 milestone's bounded queue while leaving enough task
/// slots for a saturation attempt and its receiver in the 16-task kernel.
pub(crate) const MAX_WAITING_SENDERS: usize = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpcError {
    /// The supplied task ID is out of range or names a vacant slot.
    TaskNotFound,
    /// `Reply` targeted a task that is not waiting for a reply.
    NotReplyBlocked,
    /// `Reply` was attempted by a task other than the original receiver.
    NotMessageReceiver,
    /// The receiver cannot accept another waiting sender.
    QueueFull,
    /// A raw address, length, or capacity did not satisfy the ABI rules.
    InvalidBuffer,
    /// A task attempted synchronous IPC with itself.
    WouldDeadlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutboundMessage {
    receiver: TaskId,
    message: ReadBuffer,
    reply: WriteBuffer,
}

impl OutboundMessage {
    pub(crate) const fn new(receiver: TaskId, message: ReadBuffer, reply: WriteBuffer) -> Self {
        Self {
            receiver,
            message,
            reply,
        }
    }

    pub(crate) const fn receiver(self) -> TaskId {
        self.receiver
    }

    pub(crate) const fn message(self) -> ReadBuffer {
        self.message
    }

    pub(crate) const fn reply(self) -> WriteBuffer {
        self.reply
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReceiveDestination {
    sender: TaskIdDestination,
    message: WriteBuffer,
}

impl ReceiveDestination {
    pub(crate) const fn new(sender: TaskIdDestination, message: WriteBuffer) -> Self {
        Self { sender, message }
    }

    pub(crate) const fn sender(self) -> TaskIdDestination {
        self.sender
    }

    pub(crate) const fn message(self) -> WriteBuffer {
        self.message
    }
}

#[derive(Clone, Copy)]
struct SenderQueue<const CAPACITY: usize> {
    entries: [Option<TaskId>; CAPACITY],
    head: usize,
    length: usize,
}

impl<const CAPACITY: usize> SenderQueue<CAPACITY> {
    const fn new() -> Self {
        Self {
            entries: [None; CAPACITY],
            head: 0,
            length: 0,
        }
    }

    #[cfg(test)]
    const fn len(&self) -> usize {
        self.length
    }

    fn push(&mut self, sender: TaskId) -> Result<(), IpcError> {
        if self.length == CAPACITY {
            return Err(IpcError::QueueFull);
        }

        let tail = (self.head + self.length) % CAPACITY;
        debug_assert!(self.entries[tail].is_none());
        self.entries[tail] = Some(sender);
        self.length += 1;
        Ok(())
    }

    fn pop(&mut self) -> Option<TaskId> {
        if self.length == 0 {
            return None;
        }

        let sender = self.entries[self.head]
            .take()
            .expect("occupied sender queue entry");
        self.head = (self.head + 1) % CAPACITY;
        self.length -= 1;
        Some(sender)
    }

    fn remove(&mut self, sender: TaskId) {
        let original_length = self.length;
        for _ in 0..original_length {
            let entry = self.pop().expect("sender queue length changed early");
            if entry != sender {
                self.push(entry)
                    .expect("removal cannot increase sender queue length");
            }
        }
    }

    fn clear(&mut self) {
        *self = Self::new();
    }
}

pub(crate) struct TaskIpc {
    outbound: Option<OutboundMessage>,
    receive: Option<ReceiveDestination>,
    senders: SenderQueue<MAX_WAITING_SENDERS>,
}

impl TaskIpc {
    pub(crate) const fn new() -> Self {
        Self {
            outbound: None,
            receive: None,
            senders: SenderQueue::new(),
        }
    }

    pub(crate) const fn outbound(&self) -> Option<OutboundMessage> {
        self.outbound
    }

    pub(crate) fn set_outbound(&mut self, outbound: OutboundMessage) {
        assert!(
            self.outbound.is_none(),
            "outbound IPC metadata already retained"
        );
        self.outbound = Some(outbound);
    }

    pub(crate) fn take_outbound(&mut self) -> Option<OutboundMessage> {
        self.outbound.take()
    }

    pub(crate) fn set_receive(&mut self, receive: ReceiveDestination) {
        assert!(self.receive.is_none(), "receive metadata already retained");
        self.receive = Some(receive);
    }

    pub(crate) fn take_receive(&mut self) -> Option<ReceiveDestination> {
        self.receive.take()
    }

    #[cfg(test)]
    pub(crate) const fn receive_is_pending(&self) -> bool {
        self.receive.is_some()
    }

    pub(crate) fn push_sender(&mut self, sender: TaskId) -> Result<(), IpcError> {
        self.senders.push(sender)
    }

    pub(crate) fn pop_sender(&mut self) -> Option<TaskId> {
        self.senders.pop()
    }

    pub(crate) fn remove_sender(&mut self, sender: TaskId) {
        self.senders.remove(sender);
    }

    #[cfg(test)]
    pub(crate) const fn waiting_sender_count(&self) -> usize {
        self.senders.len()
    }

    pub(crate) fn clear(&mut self) {
        self.outbound = None;
        self.receive = None;
        self.senders.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sender_queue_is_fifo_across_wraparound_and_removal() {
        let mut queue = SenderQueue::<3>::new();
        let first = TaskId::new(1);
        let second = TaskId::new(2);
        let third = TaskId::new(3);
        let fourth = TaskId::new(4);

        queue.push(first).unwrap();
        queue.push(second).unwrap();
        queue.push(third).unwrap();
        assert_eq!(queue.push(fourth), Err(IpcError::QueueFull));
        assert_eq!(queue.pop(), Some(first));
        queue.push(fourth).unwrap();
        queue.remove(third);
        assert_eq!(queue.pop(), Some(second));
        assert_eq!(queue.pop(), Some(fourth));
        assert_eq!(queue.pop(), None);
    }
}

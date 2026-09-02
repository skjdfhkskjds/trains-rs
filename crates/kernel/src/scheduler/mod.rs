//! Single-core cooperative scheduler and its stable priority policy.
//!
//! The scheduler owns every pinned task. A run begins only through
//! [`Scheduler::run`] and returns to EL1 after every task has exited or a
//! finite diagnostic reaches a deadlock.

mod priority_queue;

use core::arch::asm;
use core::cell::UnsafeCell;
use core::pin::Pin;

use trains_primitives::task::{Priority, TaskId};

use self::priority_queue::PriorityQueue;
use crate::TaskEntry;
use crate::context::{ContextSwitcher, EntryPoint, RegisterContext};
use crate::exceptions::ExceptionFrame;
use crate::ipc::IpcError;
use crate::svc::{self, CreateRequest, KernelCall, ReceiveRequest, ReplyRequest, SendRequest};
use crate::task::{Task, TaskDescriptor, TaskState};
use crate::user_memory;

const MAX_TASKS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SlotId(usize);

impl SlotId {
    const fn new(index: usize) -> Self {
        Self(index)
    }

    const fn index(self) -> usize {
        self.0
    }
}

struct TaskSlot {
    task: Option<Task>,
}

impl TaskSlot {
    const fn vacant() -> Self {
        Self { task: None }
    }

    const fn is_vacant(&self) -> bool {
        self.task.is_none()
    }

    fn occupy(&mut self, descriptor: TaskDescriptor) {
        assert!(self.is_vacant(), "cannot occupy an active task slot");
        self.task = Some(Task::new(descriptor));

        // SAFETY: the task has reached its final address in the static
        // scheduler. This slot is never moved or replaced while occupied.
        unsafe { Pin::new_unchecked(self.task.as_mut().unwrap()) }.initialize();
    }

    fn vacate(&mut self) {
        assert!(!self.is_vacant(), "cannot vacate an empty task slot");
        // Assignment drops the pinned task in place; it does not move it.
        self.task = None;
    }

    fn task_mut(&mut self) -> Pin<&mut Task> {
        // SAFETY: occupied tasks remain at stable addresses in the scheduler.
        unsafe { Pin::new_unchecked(self.task.as_mut().expect("vacant task slot")) }
    }

    fn task(&self) -> &Task {
        self.task.as_ref().expect("vacant task slot")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SchedulerPhase {
    Idle,
    Running,
}

struct SchedulerState {
    tasks: [TaskSlot; MAX_TASKS],
    ready: PriorityQueue<SlotId, Priority, MAX_TASKS>,
    current: Option<SlotId>,
    kernel_context: RegisterContext,
    phase: SchedulerPhase,
    exited_tasks: usize,
    completion_error: Option<RunError>,
}

impl SchedulerState {
    const fn new() -> Self {
        Self {
            tasks: [const { TaskSlot::vacant() }; MAX_TASKS],
            ready: PriorityQueue::new(),
            current: None,
            kernel_context: RegisterContext::empty(),
            phase: SchedulerPhase::Idle,
            exited_tasks: 0,
            completion_error: None,
        }
    }

    fn allocate_task(
        &mut self,
        parent: Option<TaskId>,
        entry: EntryPoint,
        priority: Priority,
    ) -> Result<TaskId, CreateError> {
        let slot = self
            .tasks
            .iter()
            .position(TaskSlot::is_vacant)
            .map(SlotId::new)
            .ok_or(CreateError::CapacityReached)?;
        let id = TaskId::try_from(slot.index()).map_err(|_| CreateError::TaskIdUnavailable)?;

        let descriptor = match parent {
            Some(parent) => TaskDescriptor::child(id, parent, priority, entry),
            None => TaskDescriptor::root(id, priority, entry),
        };
        self.tasks[slot.index()].occupy(descriptor);
        if self.ready.push(slot, priority).is_err() {
            self.tasks[slot.index()].vacate();
            return Err(CreateError::CapacityReached);
        }
        Ok(id)
    }

    fn create_root(&mut self, entry: TaskEntry, priority: Priority) -> Result<TaskId, CreateError> {
        if self.phase == SchedulerPhase::Running {
            return Err(CreateError::SchedulerRunning);
        }

        self.allocate_task(None, EntryPoint::new(entry as usize), priority)
    }

    fn create_child(&mut self, request: CreateRequest) -> Result<TaskId, CreateError> {
        if self.phase != SchedulerPhase::Running {
            return Err(CreateError::SchedulerRunning);
        }

        let parent = self.current_task().id();
        self.allocate_task(Some(parent), request.entry, request.priority)
    }

    fn slot_for(&self, id: TaskId) -> Option<SlotId> {
        let slot = SlotId::new(id.get() as usize);
        let task = self.tasks.get(slot.index())?.task.as_ref()?;
        (task.id() == id).then_some(slot)
    }

    fn prepare_run(&mut self) -> Result<(), RunError> {
        if self.phase == SchedulerPhase::Running {
            return Err(RunError::AlreadyRunning);
        }
        if self.ready.is_empty() {
            return Err(RunError::NoReadyTasks);
        }

        debug_assert!(self.current.is_none());
        self.phase = SchedulerPhase::Running;
        self.exited_tasks = 0;
        self.completion_error = None;
        Ok(())
    }

    fn complete_run(&mut self) -> Result<RunOutcome, RunError> {
        assert_eq!(self.phase, SchedulerPhase::Idle);
        assert!(self.current.is_none());
        assert!(self.ready.is_empty());
        debug_assert!(self.tasks.iter().all(TaskSlot::is_vacant));

        match self.completion_error.take() {
            Some(error) => Err(error),
            None => Ok(RunOutcome {
                exited_tasks: self.exited_tasks,
            }),
        }
    }

    fn save_current(&mut self, contexts: &ContextSwitcher, frame: &ExceptionFrame) {
        let current = self.current.expect("syscall without a running task");
        let mut task = self.tasks[current.index()].task_mut();
        debug_assert_eq!(task.state(), TaskState::Running);
        contexts.save(frame, task.as_mut().context_mut());
    }

    fn current_task(&self) -> &Task {
        let current = self.current.expect("operation without a running task");
        let task = self.tasks[current.index()].task();
        debug_assert_eq!(task.state(), TaskState::Running);
        task
    }

    fn select_next(&mut self) -> SlotId {
        assert!(
            self.current.is_none(),
            "cannot dispatch over a running task"
        );
        let (next, priority) = self.ready.pop().expect("no task is ready");
        let mut task = self.tasks[next.index()].task_mut();
        debug_assert_eq!(task.state(), TaskState::Ready);
        debug_assert_eq!(task.priority(), priority);
        task.as_mut().set_state(TaskState::Running);
        self.current = Some(next);
        next
    }

    fn dispatch_next(&mut self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        let next = self.select_next();
        let task = self.tasks[next.index()].task_mut();
        contexts.restore(frame, task.as_ref().context());
    }

    /// Completes a syscall and atomically makes its task ready exactly once.
    fn ready_task(&mut self, slot: SlotId, result: Option<u64>) {
        let state = self.tasks[slot.index()].task().state();
        match state {
            TaskState::Running => {
                assert_eq!(self.current, Some(slot), "running task is not current");
                self.current = None;
            }
            TaskState::SendBlocked | TaskState::ReceiveBlocked | TaskState::ReplyBlocked => {
                assert_ne!(self.current, Some(slot), "blocked task cannot be current");
            }
            TaskState::Ready => panic!("cannot enqueue a ready task twice"),
        }

        let mut task = self.tasks[slot.index()].task_mut();
        if let Some(result) = result {
            task.as_mut().context_mut().set_register(0, result);
        }
        task.as_mut().set_state(TaskState::Ready);
        let priority = task.priority();
        self.ready
            .push(slot, priority)
            .expect("a non-ready task leaves one ready-queue position free");
    }

    fn ready_current(&mut self, result: Option<u64>) {
        let current = self.current.expect("ready without a running task");
        self.ready_task(current, result);
    }

    fn block_current(&mut self, state: TaskState) {
        assert!(
            matches!(
                state,
                TaskState::SendBlocked | TaskState::ReceiveBlocked | TaskState::ReplyBlocked
            ),
            "running task can only enter a blocking state"
        );
        let current = self.current.take().expect("block without a running task");
        let mut task = self.tasks[current.index()].task_mut();
        debug_assert_eq!(task.state(), TaskState::Running);
        task.as_mut().set_state(state);
    }

    fn transition_blocked(&mut self, slot: SlotId, from: TaskState, to: TaskState) {
        let mut task = self.tasks[slot.index()].task_mut();
        assert_eq!(task.state(), from, "unexpected blocked-task transition");
        task.as_mut().set_state(to);
    }

    /// Ends a finite run if no task can execute.
    ///
    /// An empty task table is normal completion. Remaining blocked tasks are a
    /// deadlock; their retained user pointers are discarded and all slots are
    /// vacated so a later diagnostic can start from a clean scheduler.
    fn finish_if_stalled(&mut self) -> bool {
        if !self.ready.is_empty() {
            return false;
        }

        assert!(
            self.current.is_none(),
            "running task with an empty ready queue"
        );
        let deadlocked = self.tasks.iter().any(|slot| !slot.is_vacant());
        if deadlocked {
            self.completion_error = Some(RunError::Deadlock);
            for slot in &mut self.tasks {
                if !slot.is_vacant() {
                    slot.task_mut().ipc_mut().clear();
                    slot.vacate();
                }
            }
        }
        self.phase = SchedulerPhase::Idle;
        true
    }

    fn resume_or_finish(&mut self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        if self.finish_if_stalled() {
            contexts.restore(frame, &self.kernel_context);
        } else {
            self.dispatch_next(contexts, frame);
        }
    }

    fn complete_current_call(
        &mut self,
        contexts: &ContextSwitcher,
        frame: &mut ExceptionFrame,
        result: Option<u64>,
    ) {
        self.save_current(contexts, frame);
        self.ready_current(result);
        self.resume_or_finish(contexts, frame);
    }

    fn start(&mut self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        assert_eq!(self.phase, SchedulerPhase::Running);
        assert!(self.current.is_none());
        contexts.save(frame, &mut self.kernel_context);
        self.dispatch_next(contexts, frame);
    }

    fn yield_current(&mut self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        self.complete_current_call(contexts, frame, None);
    }

    fn create_current(
        &mut self,
        contexts: &ContextSwitcher,
        frame: &mut ExceptionFrame,
        request: Result<CreateRequest, CreateError>,
    ) {
        self.save_current(contexts, frame);
        let result = request.and_then(|request| self.create_child(request));
        self.ready_current(Some(svc::encode_create_result(result)));
        self.resume_or_finish(contexts, frame);
    }

    fn current_id(&mut self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        let id = self.current_task().id();
        self.complete_current_call(contexts, frame, Some(u64::from(id)));
    }

    fn current_parent_id(&mut self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        let parent = self.current_task().parent();
        self.complete_current_call(contexts, frame, Some(svc::encode_parent(parent)));
    }

    fn complete_current_ipc_error(&mut self, error: IpcError) {
        self.ready_current(Some(svc::encode_ipc_result(Err(error))));
    }

    fn send_current_transition(&mut self, request: Result<SendRequest, IpcError>) {
        let request = match request {
            Ok(request) => request,
            Err(error) => {
                self.complete_current_ipc_error(error);
                return;
            }
        };
        let sender = self.current.expect("Send without a running task");
        let sender_id = self.tasks[sender.index()].task().id();
        let receiver = match self.slot_for(request.receiver) {
            Some(receiver) => receiver,
            None => {
                self.complete_current_ipc_error(IpcError::TaskNotFound);
                return;
            }
        };
        if sender == receiver {
            self.complete_current_ipc_error(IpcError::WouldDeadlock);
            return;
        }

        if self.tasks[receiver.index()].task().state() == TaskState::ReceiveBlocked {
            let destination = self.tasks[receiver.index()]
                .task_mut()
                .ipc_mut()
                .take_receive()
                .expect("receive-blocked task has no destination");
            self.tasks[sender.index()]
                .task_mut()
                .ipc_mut()
                .set_outbound(request.outbound());

            user_memory::copy(request.message, destination.message());
            user_memory::write_task_id(destination.sender(), sender_id);
            self.ready_task(
                receiver,
                Some(svc::encode_ipc_result(Ok(request.message.length()))),
            );
            self.block_current(TaskState::ReplyBlocked);
            return;
        }

        if let Err(error) = self.tasks[receiver.index()]
            .task_mut()
            .ipc_mut()
            .push_sender(sender_id)
        {
            self.complete_current_ipc_error(error);
            return;
        }
        self.tasks[sender.index()]
            .task_mut()
            .ipc_mut()
            .set_outbound(request.outbound());
        self.block_current(TaskState::SendBlocked);
    }

    fn receive_current_transition(&mut self, request: Result<ReceiveRequest, IpcError>) {
        let request = match request {
            Ok(request) => request,
            Err(error) => {
                self.complete_current_ipc_error(error);
                return;
            }
        };
        let receiver = self.current.expect("Receive without a running task");
        let receiver_id = self.tasks[receiver.index()].task().id();

        let sender = loop {
            let sender_id = match self.tasks[receiver.index()]
                .task_mut()
                .ipc_mut()
                .pop_sender()
            {
                Some(sender) => sender,
                None => break None,
            };
            let Some(sender) = self.slot_for(sender_id) else {
                continue;
            };
            let task = self.tasks[sender.index()].task();
            let is_valid = task.state() == TaskState::SendBlocked
                && task
                    .ipc()
                    .outbound()
                    .is_some_and(|outbound| outbound.receiver() == receiver_id);
            if is_valid {
                break Some(sender);
            }
        };

        let Some(sender) = sender else {
            self.tasks[receiver.index()]
                .task_mut()
                .ipc_mut()
                .set_receive(request.destination());
            self.block_current(TaskState::ReceiveBlocked);
            return;
        };

        let sender_id = self.tasks[sender.index()].task().id();
        let outbound = self.tasks[sender.index()]
            .task()
            .ipc()
            .outbound()
            .expect("send-blocked task has no outbound message");
        user_memory::copy(outbound.message(), request.message);
        user_memory::write_task_id(request.sender, sender_id);
        self.transition_blocked(sender, TaskState::SendBlocked, TaskState::ReplyBlocked);
        self.ready_current(Some(svc::encode_ipc_result(Ok(outbound
            .message()
            .length()))));
    }

    fn reply_current_transition(&mut self, request: Result<ReplyRequest, IpcError>) {
        let request = match request {
            Ok(request) => request,
            Err(error) => {
                self.complete_current_ipc_error(error);
                return;
            }
        };
        let replier = self.current.expect("Reply without a running task");
        let replier_id = self.tasks[replier.index()].task().id();
        let sender = match self.slot_for(request.sender) {
            Some(sender) => sender,
            None => {
                self.complete_current_ipc_error(IpcError::TaskNotFound);
                return;
            }
        };
        if self.tasks[sender.index()].task().state() != TaskState::ReplyBlocked {
            self.complete_current_ipc_error(IpcError::NotReplyBlocked);
            return;
        }
        let outbound = self.tasks[sender.index()]
            .task()
            .ipc()
            .outbound()
            .expect("reply-blocked task has no outbound message");
        if outbound.receiver() != replier_id {
            self.complete_current_ipc_error(IpcError::NotMessageReceiver);
            return;
        }

        user_memory::copy(request.reply, outbound.reply());
        self.tasks[sender.index()]
            .task_mut()
            .ipc_mut()
            .take_outbound()
            .expect("reply completion lost outbound metadata");
        let result = svc::encode_ipc_result(Ok(request.reply.length()));
        self.ready_task(sender, Some(result));
        self.ready_current(Some(result));
    }

    fn send_current(
        &mut self,
        contexts: &ContextSwitcher,
        frame: &mut ExceptionFrame,
        request: Result<SendRequest, IpcError>,
    ) {
        self.save_current(contexts, frame);
        self.send_current_transition(request);
        self.resume_or_finish(contexts, frame);
    }

    fn receive_current(
        &mut self,
        contexts: &ContextSwitcher,
        frame: &mut ExceptionFrame,
        request: Result<ReceiveRequest, IpcError>,
    ) {
        self.save_current(contexts, frame);
        self.receive_current_transition(request);
        self.resume_or_finish(contexts, frame);
    }

    fn reply_current(
        &mut self,
        contexts: &ContextSwitcher,
        frame: &mut ExceptionFrame,
        request: Result<ReplyRequest, IpcError>,
    ) {
        self.save_current(contexts, frame);
        self.reply_current_transition(request);
        self.resume_or_finish(contexts, frame);
    }

    /// Cleans every kernel-owned IPC reference to the exiting task, then
    /// vacates it without ever placing its context back in a queue.
    fn vacate_current(&mut self) {
        let current = self.current.expect("exit without a running task");
        let exiting_id = self.tasks[current.index()].task().id();
        let mut wake = [None; MAX_TASKS];
        let mut wake_count = 0;

        for index in 0..MAX_TASKS {
            let slot = SlotId::new(index);
            if slot == current || self.tasks[index].is_vacant() {
                continue;
            }

            let mut task = self.tasks[index].task_mut();
            task.as_mut().ipc_mut().remove_sender(exiting_id);
            let targets_exiting = task
                .ipc()
                .outbound()
                .is_some_and(|outbound| outbound.receiver() == exiting_id);
            if targets_exiting {
                task.as_mut().ipc_mut().take_outbound();
                if matches!(
                    task.state(),
                    TaskState::SendBlocked | TaskState::ReplyBlocked
                ) {
                    wake[wake_count] = Some(slot);
                    wake_count += 1;
                }
            }
        }

        for slot in wake.into_iter().take(wake_count).flatten() {
            self.ready_task(
                slot,
                Some(svc::encode_ipc_result(Err(IpcError::TaskNotFound))),
            );
        }

        self.tasks[current.index()].task_mut().ipc_mut().clear();
        self.current = None;
        self.tasks[current.index()].vacate();
        self.exited_tasks += 1;
    }

    fn exit_current(&mut self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        self.vacate_current();
        self.resume_or_finish(contexts, frame);
    }
}

/// Synchronized access to the single-core scheduler state.
pub(crate) struct Scheduler {
    state: UnsafeCell<SchedulerState>,
}

// SAFETY: only the primary core runs kernel code, and exception entry masks
// IRQs before scheduler mutation. `with_state` bounds every mutable borrow.
unsafe impl Sync for Scheduler {}

pub(crate) static SCHEDULER: Scheduler = Scheduler {
    state: UnsafeCell::new(SchedulerState::new()),
};

impl Scheduler {
    fn with_state<R>(&self, operation: impl FnOnce(&mut SchedulerState) -> R) -> R {
        // SAFETY: the scheduler's single-core, IRQ-masked access invariant
        // ensures each call has exclusive access for its duration.
        operation(unsafe { &mut *self.state.get() })
    }

    pub(crate) fn create(
        &self,
        entry: TaskEntry,
        priority: Priority,
    ) -> Result<TaskId, CreateError> {
        self.with_state(|scheduler| scheduler.create_root(entry, priority))
    }

    pub(crate) fn run(&self) -> Result<RunOutcome, RunError> {
        self.with_state(SchedulerState::prepare_run)?;

        // SAFETY: the handler saves this EL1 context, dispatches the first
        // ready EL0 task, and restores it after the final task exits.
        unsafe { asm!("svc #{svc}", svc = const KernelCall::StartScheduler as u16) };

        self.with_state(SchedulerState::complete_run)
    }

    pub(crate) fn start_from(&self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        self.with_state(|scheduler| scheduler.start(contexts, frame));
    }

    pub(crate) fn yield_current(&self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        self.with_state(|scheduler| scheduler.yield_current(contexts, frame));
    }

    pub(crate) fn create_current(
        &self,
        contexts: &ContextSwitcher,
        frame: &mut ExceptionFrame,
        request: Result<CreateRequest, CreateError>,
    ) {
        self.with_state(|scheduler| scheduler.create_current(contexts, frame, request));
    }

    pub(crate) fn current_id(&self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        self.with_state(|scheduler| scheduler.current_id(contexts, frame));
    }

    pub(crate) fn current_parent_id(&self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        self.with_state(|scheduler| scheduler.current_parent_id(contexts, frame));
    }

    pub(crate) fn send_current(
        &self,
        contexts: &ContextSwitcher,
        frame: &mut ExceptionFrame,
        request: Result<SendRequest, IpcError>,
    ) {
        self.with_state(|scheduler| scheduler.send_current(contexts, frame, request));
    }

    pub(crate) fn receive_current(
        &self,
        contexts: &ContextSwitcher,
        frame: &mut ExceptionFrame,
        request: Result<ReceiveRequest, IpcError>,
    ) {
        self.with_state(|scheduler| scheduler.receive_current(contexts, frame, request));
    }

    pub(crate) fn reply_current(
        &self,
        contexts: &ContextSwitcher,
        frame: &mut ExceptionFrame,
        request: Result<ReplyRequest, IpcError>,
    ) {
        self.with_state(|scheduler| scheduler.reply_current(contexts, frame, request));
    }

    pub(crate) fn exit_current(&self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        self.with_state(|scheduler| scheduler.exit_current(contexts, frame));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateError {
    InvalidPriority,
    CapacityReached,
    SchedulerRunning,
    TaskIdUnavailable,
    InvalidEntryPoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunError {
    AlreadyRunning,
    NoReadyTasks,
    /// Every remaining task was blocked and no task could make progress.
    Deadlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunOutcome {
    exited_tasks: usize,
}

impl RunOutcome {
    pub const fn exited_tasks(self) -> usize {
        self.exited_tasks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::MAX_WAITING_SENDERS;

    extern "C" fn task_entry(_id: TaskId) -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    fn request(priority: u32) -> CreateRequest {
        CreateRequest {
            entry: EntryPoint::new(task_entry as *const () as usize),
            priority: Priority::new(priority),
        }
    }

    fn selected_id(state: &mut SchedulerState) -> TaskId {
        let slot = state.select_next();
        state.tasks[slot.index()].task().id()
    }

    fn saved_x0(state: &SchedulerState, id: TaskId) -> u64 {
        let task = state.tasks[id.get() as usize].task();
        // SAFETY: an occupied scheduler slot is never moved while its task is
        // alive, including for the duration of these state-machine tests.
        unsafe { Pin::new_unchecked(task) }.context().register(0)
    }

    fn task(state: &SchedulerState, id: TaskId) -> &Task {
        state.tasks[id.get() as usize].task()
    }

    fn send_request(receiver: TaskId, message: &[u8], reply: &mut [u8]) -> SendRequest {
        SendRequest::decode(
            u64::from(receiver),
            message.as_ptr() as usize as u64,
            message.len() as u64,
            reply.as_mut_ptr() as usize as u64,
            reply.len() as u64,
        )
        .unwrap()
    }

    fn receive_request(sender: &mut TaskId, message: &mut [u8]) -> ReceiveRequest {
        ReceiveRequest::decode(
            sender as *mut TaskId as usize as u64,
            message.as_mut_ptr() as usize as u64,
            message.len() as u64,
        )
        .unwrap()
    }

    fn reply_request(sender: TaskId, reply: &[u8]) -> ReplyRequest {
        ReplyRequest::decode(
            u64::from(sender),
            reply.as_ptr() as usize as u64,
            reply.len() as u64,
        )
        .unwrap()
    }

    fn saved_ipc_result(state: &SchedulerState, id: TaskId) -> Result<usize, IpcError> {
        svc::decode_ipc_result(saved_x0(state, id))
    }

    #[test]
    fn equal_priority_tasks_remain_fifo_across_yields() {
        let mut state = SchedulerState::new();
        let low = state.create_root(task_entry, Priority::new(3)).unwrap();
        let first = state.create_root(task_entry, Priority::new(1)).unwrap();
        let second = state.create_root(task_entry, Priority::new(1)).unwrap();

        state.prepare_run().unwrap();
        assert_eq!(selected_id(&mut state), first);
        state.ready_current(None);
        assert_eq!(selected_id(&mut state), second);
        state.ready_current(None);
        assert_eq!(selected_id(&mut state), first);
        assert_eq!(state.ready.len(), 2);
        assert_eq!(
            state.tasks[low.get() as usize].task().state(),
            TaskState::Ready
        );
    }

    #[test]
    fn child_records_parent_and_preempts_it_by_priority() {
        let mut state = SchedulerState::new();
        let parent = state.create_root(task_entry, Priority::new(2)).unwrap();
        state.prepare_run().unwrap();
        assert_eq!(selected_id(&mut state), parent);

        let child = state.create_child(request(1)).unwrap();
        state.ready_current(Some(svc::encode_create_result(Ok(child))));

        assert_eq!(saved_x0(&state, parent), u64::from(child));
        assert_eq!(selected_id(&mut state), child);
        assert_eq!(state.current_task().parent(), Some(parent));
    }

    #[test]
    fn full_storage_fails_without_changing_ready_queue_order() {
        let mut state = SchedulerState::new();
        for expected_id in 0..MAX_TASKS {
            let priority = if expected_id == 0 { 0 } else { 1 };
            let id = state
                .create_root(task_entry, Priority::new(priority))
                .unwrap();
            assert_eq!(id.get() as usize, expected_id);
        }

        state.prepare_run().unwrap();
        let parent = selected_id(&mut state);
        assert_eq!(state.ready.len(), MAX_TASKS - 1);
        assert_eq!(
            state.create_child(request(0)),
            Err(CreateError::CapacityReached)
        );
        assert_eq!(state.ready.len(), MAX_TASKS - 1);

        state.ready_current(Some(svc::encode_create_result(Err(
            CreateError::CapacityReached,
        ))));
        assert_eq!(state.ready.len(), MAX_TASKS);
        assert_eq!(selected_id(&mut state), parent);
        assert_eq!(state.ready.len(), MAX_TASKS - 1);
        assert_eq!(
            svc::decode_create_result(saved_x0(&state, parent)),
            Err(CreateError::CapacityReached)
        );
    }

    #[test]
    fn exiting_task_is_not_selected_and_its_slot_is_reused() {
        let mut state = SchedulerState::new();
        let exiting = state.create_root(task_entry, Priority::new(1)).unwrap();
        let survivor = state.create_root(task_entry, Priority::new(2)).unwrap();
        state.prepare_run().unwrap();
        assert_eq!(selected_id(&mut state), exiting);

        state.vacate_current();
        assert!(state.tasks[exiting.get() as usize].is_vacant());
        assert_eq!(selected_id(&mut state), survivor);

        let replacement = state.create_child(request(1)).unwrap();
        assert_eq!(replacement, exiting);
        state.ready_current(Some(svc::encode_create_result(Ok(replacement))));
        assert_eq!(selected_id(&mut state), replacement);
        assert_eq!(state.current_task().parent(), Some(survivor));
        assert_eq!(state.exited_tasks, 1);
    }

    #[test]
    fn bootstrap_task_has_no_parent() {
        let mut state = SchedulerState::new();
        let root = state.create_root(task_entry, Priority::new(0)).unwrap();

        assert_eq!(state.tasks[root.get() as usize].task().parent(), None);
        assert_eq!(svc::decode_parent(svc::encode_parent(None)), None);
    }

    #[test]
    fn send_first_blocks_until_receive_and_reply_with_logical_lengths() {
        let mut state = SchedulerState::new();
        let sender = state.create_root(task_entry, Priority::new(0)).unwrap();
        let receiver = state.create_root(task_entry, Priority::new(1)).unwrap();
        let message = *b"four";
        let mut reply_buffer = [0xa5; 2];

        state.prepare_run().unwrap();
        assert_eq!(selected_id(&mut state), sender);
        state.send_current_transition(Ok(send_request(receiver, &message, &mut reply_buffer)));
        assert_eq!(task(&state, sender).state(), TaskState::SendBlocked);
        assert_eq!(task(&state, receiver).ipc().waiting_sender_count(), 1);

        assert_eq!(selected_id(&mut state), receiver);
        let mut sender_out = TaskId::new(u32::MAX);
        let mut received = [0xa5; 3];
        state.receive_current_transition(Ok(receive_request(&mut sender_out, &mut received)));
        assert_eq!(sender_out, sender);
        assert_eq!(received, *b"fou");
        assert_eq!(saved_ipc_result(&state, receiver), Ok(message.len()));
        assert_eq!(task(&state, sender).state(), TaskState::ReplyBlocked);
        assert_eq!(task(&state, receiver).ipc().waiting_sender_count(), 0);

        assert_eq!(selected_id(&mut state), receiver);
        let reply = *b"yes";
        state.reply_current_transition(Ok(reply_request(sender, &reply)));
        assert_eq!(reply_buffer, *b"ye");
        assert_eq!(saved_ipc_result(&state, sender), Ok(reply.len()));
        assert_eq!(saved_ipc_result(&state, receiver), Ok(reply.len()));
        assert!(task(&state, sender).ipc().outbound().is_none());
        assert_eq!(selected_id(&mut state), sender);
    }

    #[test]
    fn receive_first_retains_destinations_until_a_sender_arrives() {
        let mut state = SchedulerState::new();
        let receiver = state.create_root(task_entry, Priority::new(0)).unwrap();
        let sender = state.create_root(task_entry, Priority::new(1)).unwrap();
        let mut sender_out = TaskId::new(u32::MAX);
        let mut received = [0; 4];

        state.prepare_run().unwrap();
        assert_eq!(selected_id(&mut state), receiver);
        state.receive_current_transition(Ok(receive_request(&mut sender_out, &mut received)));
        assert_eq!(task(&state, receiver).state(), TaskState::ReceiveBlocked);

        assert_eq!(selected_id(&mut state), sender);
        let message = *b"ping";
        let mut reply = [0; 1];
        state.send_current_transition(Ok(send_request(receiver, &message, &mut reply)));
        assert_eq!(sender_out, sender);
        assert_eq!(received, message);
        assert_eq!(saved_ipc_result(&state, receiver), Ok(message.len()));
        assert!(!task(&state, receiver).ipc().receive_is_pending());
        assert_eq!(task(&state, sender).state(), TaskState::ReplyBlocked);
        assert_eq!(selected_id(&mut state), receiver);
    }

    #[test]
    fn multiple_senders_are_received_in_fifo_order() {
        let mut state = SchedulerState::new();
        let receiver = state.create_root(task_entry, Priority::new(2)).unwrap();
        let first = state.create_root(task_entry, Priority::new(0)).unwrap();
        let second = state.create_root(task_entry, Priority::new(0)).unwrap();
        let first_message = [1];
        let second_message = [2];
        let mut first_reply = [0];
        let mut second_reply = [0];

        state.prepare_run().unwrap();
        assert_eq!(selected_id(&mut state), first);
        state.send_current_transition(Ok(send_request(receiver, &first_message, &mut first_reply)));
        assert_eq!(selected_id(&mut state), second);
        state.send_current_transition(Ok(send_request(
            receiver,
            &second_message,
            &mut second_reply,
        )));
        assert_eq!(selected_id(&mut state), receiver);

        for (expected_sender, expected_byte) in [(first, 1), (second, 2)] {
            let mut sender_out = TaskId::new(u32::MAX);
            let mut received = [0];
            state.receive_current_transition(Ok(receive_request(&mut sender_out, &mut received)));
            assert_eq!(sender_out, expected_sender);
            assert_eq!(received, [expected_byte]);
            assert_eq!(
                task(&state, expected_sender).state(),
                TaskState::ReplyBlocked
            );
            assert_eq!(selected_id(&mut state), receiver);
        }
        assert_eq!(task(&state, receiver).ipc().waiting_sender_count(), 0);
    }

    #[test]
    fn ipc_errors_are_typed_and_every_failure_requeues_the_caller() {
        let mut state = SchedulerState::new();
        let caller = state.create_root(task_entry, Priority::new(0)).unwrap();
        let target = state.create_root(task_entry, Priority::new(1)).unwrap();
        let message = [1];
        let mut reply = [0];

        state.prepare_run().unwrap();
        assert_eq!(selected_id(&mut state), caller);
        state.send_current_transition(Ok(send_request(
            TaskId::new((MAX_TASKS - 1) as u32),
            &message,
            &mut reply,
        )));
        assert_eq!(
            saved_ipc_result(&state, caller),
            Err(IpcError::TaskNotFound)
        );
        assert_eq!(task(&state, caller).state(), TaskState::Ready);
        assert!(task(&state, caller).ipc().outbound().is_none());
        assert_eq!(selected_id(&mut state), caller);

        state.reply_current_transition(Ok(reply_request(target, &message)));
        assert_eq!(
            saved_ipc_result(&state, caller),
            Err(IpcError::NotReplyBlocked)
        );
        assert_eq!(task(&state, caller).state(), TaskState::Ready);
        assert_eq!(selected_id(&mut state), caller);

        state.send_current_transition(Ok(send_request(caller, &message, &mut reply)));
        assert_eq!(
            saved_ipc_result(&state, caller),
            Err(IpcError::WouldDeadlock)
        );
        assert_eq!(task(&state, caller).state(), TaskState::Ready);
    }

    #[test]
    fn only_the_original_receiver_may_reply() {
        let mut state = SchedulerState::new();
        let sender = state.create_root(task_entry, Priority::new(0)).unwrap();
        let receiver = state.create_root(task_entry, Priority::new(1)).unwrap();
        let third_party = state.create_root(task_entry, Priority::new(2)).unwrap();
        let message = [1];
        let mut sender_reply = [0];
        let mut nested_reply = [0];

        state.prepare_run().unwrap();
        assert_eq!(selected_id(&mut state), sender);
        state.send_current_transition(Ok(send_request(receiver, &message, &mut sender_reply)));
        assert_eq!(selected_id(&mut state), receiver);
        let mut sender_out = TaskId::new(u32::MAX);
        let mut received = [0];
        state.receive_current_transition(Ok(receive_request(&mut sender_out, &mut received)));
        assert_eq!(selected_id(&mut state), receiver);

        state.send_current_transition(Ok(send_request(third_party, &message, &mut nested_reply)));
        assert_eq!(selected_id(&mut state), third_party);
        state.reply_current_transition(Ok(reply_request(sender, &message)));
        assert_eq!(
            saved_ipc_result(&state, third_party),
            Err(IpcError::NotMessageReceiver)
        );
        assert_eq!(task(&state, sender).state(), TaskState::ReplyBlocked);
        assert!(task(&state, sender).ipc().outbound().is_some());
    }

    #[test]
    fn full_sender_queue_fails_without_metadata_or_fifo_corruption() {
        let mut state = SchedulerState::new();
        let receiver = state.create_root(task_entry, Priority::new(1)).unwrap();
        let mut senders = [TaskId::new(0); MAX_WAITING_SENDERS + 1];
        for sender in &mut senders {
            *sender = state.create_root(task_entry, Priority::new(0)).unwrap();
        }
        let messages: [[u8; 1]; MAX_WAITING_SENDERS + 1] =
            core::array::from_fn(|index| [index as u8]);
        let mut replies = [[0]; MAX_WAITING_SENDERS + 1];

        state.prepare_run().unwrap();
        for index in 0..MAX_WAITING_SENDERS {
            assert_eq!(selected_id(&mut state), senders[index]);
            state.send_current_transition(Ok(send_request(
                receiver,
                &messages[index],
                &mut replies[index],
            )));
        }
        assert_eq!(
            task(&state, receiver).ipc().waiting_sender_count(),
            MAX_WAITING_SENDERS
        );

        let rejected = senders[MAX_WAITING_SENDERS];
        assert_eq!(selected_id(&mut state), rejected);
        state.send_current_transition(Ok(send_request(
            receiver,
            &messages[MAX_WAITING_SENDERS],
            &mut replies[MAX_WAITING_SENDERS],
        )));
        assert_eq!(saved_ipc_result(&state, rejected), Err(IpcError::QueueFull));
        assert!(task(&state, rejected).ipc().outbound().is_none());
        assert_eq!(
            task(&state, receiver).ipc().waiting_sender_count(),
            MAX_WAITING_SENDERS
        );

        assert_eq!(selected_id(&mut state), rejected);
        state.vacate_current();
        assert_eq!(selected_id(&mut state), receiver);
        for index in 0..MAX_WAITING_SENDERS {
            let mut sender_out = TaskId::new(u32::MAX);
            let mut received = [u8::MAX];
            state.receive_current_transition(Ok(receive_request(&mut sender_out, &mut received)));
            assert_eq!(sender_out, senders[index]);
            assert_eq!(received, messages[index]);
            assert_eq!(selected_id(&mut state), receiver);
        }
    }

    #[test]
    fn receiver_exit_wakes_send_and_reply_blocked_senders() {
        let mut state = SchedulerState::new();
        let first = state.create_root(task_entry, Priority::new(0)).unwrap();
        let second = state.create_root(task_entry, Priority::new(1)).unwrap();
        let receiver = state.create_root(task_entry, Priority::new(2)).unwrap();
        let first_message = [1];
        let second_message = [2];
        let mut first_reply = [0];
        let mut second_reply = [0];

        state.prepare_run().unwrap();
        assert_eq!(selected_id(&mut state), first);
        state.send_current_transition(Ok(send_request(receiver, &first_message, &mut first_reply)));
        assert_eq!(selected_id(&mut state), second);
        state.send_current_transition(Ok(send_request(
            receiver,
            &second_message,
            &mut second_reply,
        )));
        assert_eq!(selected_id(&mut state), receiver);
        let mut sender_out = TaskId::new(u32::MAX);
        let mut received = [0];
        state.receive_current_transition(Ok(receive_request(&mut sender_out, &mut received)));
        assert_eq!(sender_out, first);
        assert_eq!(selected_id(&mut state), receiver);
        assert_eq!(task(&state, first).state(), TaskState::ReplyBlocked);
        assert_eq!(task(&state, second).state(), TaskState::SendBlocked);

        state.vacate_current();
        assert!(state.tasks[receiver.get() as usize].is_vacant());
        for sender in [first, second] {
            assert_eq!(
                saved_ipc_result(&state, sender),
                Err(IpcError::TaskNotFound)
            );
            assert_eq!(task(&state, sender).state(), TaskState::Ready);
            assert!(task(&state, sender).ipc().outbound().is_none());
        }
        assert_eq!(selected_id(&mut state), first);
    }

    #[test]
    fn blocked_cycle_finishes_as_a_clean_recoverable_deadlock() {
        let mut state = SchedulerState::new();
        let first = state.create_root(task_entry, Priority::new(0)).unwrap();
        let second = state.create_root(task_entry, Priority::new(1)).unwrap();
        let first_message = [1];
        let second_message = [2];
        let mut first_reply = [0];
        let mut second_reply = [0];

        state.prepare_run().unwrap();
        assert_eq!(selected_id(&mut state), first);
        state.send_current_transition(Ok(send_request(second, &first_message, &mut first_reply)));
        assert_eq!(selected_id(&mut state), second);
        state.send_current_transition(Ok(send_request(first, &second_message, &mut second_reply)));
        assert!(state.current.is_none());
        assert!(state.ready.is_empty());
        assert!(state.finish_if_stalled());
        assert_eq!(state.complete_run(), Err(RunError::Deadlock));
        assert!(state.tasks.iter().all(TaskSlot::is_vacant));

        let replacement = state.create_root(task_entry, Priority::new(0)).unwrap();
        assert_eq!(replacement, TaskId::new(0));
        assert_eq!(state.prepare_run(), Ok(()));
    }
}

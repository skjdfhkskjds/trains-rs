//! Single-core cooperative scheduler and its stable priority policy.
//!
//! The scheduler owns every pinned task. A run begins only through
//! [`Scheduler::run`] and returns to EL1 after every task has exited.

mod priority_queue;

use core::arch::asm;
use core::cell::UnsafeCell;
use core::pin::Pin;

use trains_primitives::task::{Priority, TaskId};

use self::priority_queue::PriorityQueue;
use crate::TaskEntry;
use crate::context::{ContextSwitcher, EntryPoint, RegisterContext};
use crate::exceptions::ExceptionFrame;
use crate::svc::{self, CreateRequest, KernelCall};
use crate::task::{Task, TaskDescriptor, TaskState};

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
        Ok(())
    }

    fn complete_run(&mut self) -> RunOutcome {
        assert_eq!(self.phase, SchedulerPhase::Idle);
        assert!(self.current.is_none());
        assert!(self.ready.is_empty());
        debug_assert!(self.tasks.iter().all(TaskSlot::is_vacant));

        RunOutcome {
            exited_tasks: self.exited_tasks,
        }
    }

    fn save_current(&mut self, contexts: &ContextSwitcher, frame: &ExceptionFrame) {
        let current = self.current.expect("yield without a running task");
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

    /// Moves the running task to `Ready`, optionally completing its syscall.
    ///
    /// This is the only transition that re-enqueues a running task, which
    /// keeps result writes, task state, and ready-queue membership atomic.
    fn ready_current(&mut self, result: Option<u64>) {
        let current = self.current.take().expect("ready without a running task");
        let mut task = self.tasks[current.index()].task_mut();
        debug_assert_eq!(task.state(), TaskState::Running);
        if let Some(result) = result {
            task.as_mut().context_mut().set_register(0, result);
        }
        task.as_mut().set_state(TaskState::Ready);
        let priority = task.priority();
        self.ready
            .push(current, priority)
            .expect("the running task leaves one ready-queue position free");
    }

    fn complete_current_call(
        &mut self,
        contexts: &ContextSwitcher,
        frame: &mut ExceptionFrame,
        result: Option<u64>,
    ) {
        self.save_current(contexts, frame);
        self.ready_current(result);
        self.dispatch_next(contexts, frame);
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
        self.dispatch_next(contexts, frame);
    }

    fn current_id(&mut self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        let id = self.current_task().id();
        self.complete_current_call(contexts, frame, Some(u64::from(id)));
    }

    fn current_parent_id(&mut self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        let parent = self.current_task().parent();
        self.complete_current_call(contexts, frame, Some(svc::encode_parent(parent)));
    }

    /// Vacates the running task without ever placing it back in a queue.
    fn vacate_current(&mut self) {
        let current = self.current.take().expect("exit without a running task");
        self.tasks[current.index()].vacate();
        self.exited_tasks += 1;
    }

    fn exit_current(&mut self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        self.vacate_current();

        if self.ready.is_empty() {
            self.phase = SchedulerPhase::Idle;
            contexts.restore(frame, &self.kernel_context);
        } else {
            self.dispatch_next(contexts, frame);
        }
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

        Ok(self.with_state(SchedulerState::complete_run))
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
}

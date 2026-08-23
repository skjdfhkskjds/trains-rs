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
use crate::context::{ContextSwitcher, RegisterContext};
use crate::exceptions::ExceptionFrame;
use crate::svc::KernelCall;
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

    fn create(&mut self, entry: TaskEntry, priority: Priority) -> Result<TaskId, CreateError> {
        if self.phase == SchedulerPhase::Running {
            return Err(CreateError::SchedulerRunning);
        }

        let slot = self
            .tasks
            .iter()
            .position(TaskSlot::is_vacant)
            .map(SlotId::new)
            .ok_or(CreateError::CapacityReached)?;
        let id = TaskId::try_from(slot.index()).map_err(|_| CreateError::TaskIdUnavailable)?;

        self.tasks[slot.index()].occupy(TaskDescriptor::root(id, priority, entry));
        self.ready
            .push(slot, priority)
            .expect("a vacant task slot guarantees ready-queue capacity");
        Ok(id)
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

    fn dispatch_next(&mut self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        let (next, priority) = self.ready.pop().expect("no task is ready");
        let mut task = self.tasks[next.index()].task_mut();
        debug_assert_eq!(task.state(), TaskState::Ready);
        debug_assert_eq!(task.priority(), priority);
        task.as_mut().set_state(TaskState::Running);
        self.current = Some(next);
        contexts.restore(frame, task.as_ref().context());
    }

    fn start(&mut self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        assert_eq!(self.phase, SchedulerPhase::Running);
        assert!(self.current.is_none());
        contexts.save(frame, &mut self.kernel_context);
        self.dispatch_next(contexts, frame);
    }

    fn yield_current(&mut self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        self.save_current(contexts, frame);
        let current = self.current.take().expect("yield without a running task");
        let mut task = self.tasks[current.index()].task_mut();
        task.as_mut().set_state(TaskState::Ready);
        let priority = task.priority();
        self.ready
            .push(current, priority)
            .expect("the running task leaves one ready-queue position free");
        self.dispatch_next(contexts, frame);
    }

    fn exit_current(&mut self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        let current = self.current.take().expect("exit without a running task");
        self.tasks[current.index()].vacate();
        self.exited_tasks += 1;

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
        self.with_state(|scheduler| scheduler.create(entry, priority))
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

    pub(crate) fn exit_current(&self, contexts: &ContextSwitcher, frame: &mut ExceptionFrame) {
        self.with_state(|scheduler| scheduler.exit_current(contexts, frame));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateError {
    CapacityReached,
    SchedulerRunning,
    TaskIdUnavailable,
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

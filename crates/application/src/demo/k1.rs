//! End-to-end K1 task lifecycle and scheduling diagnostic.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use trains_kernel::{
    CurrentTask, Kernel, Priority, RunOutcome, TaskEntry, TaskId,
    runtime::{Command, CommandResult},
};
use trains_platform::Raspi4;

use crate::KERNEL;

const CHILD_COUNT: usize = 4;
const TASK_COUNT: usize = CHILD_COUNT + 1;
const TRACE_LENGTH: usize = 18;
const UNSET_ID: u32 = u32::MAX;
const NO_CHILD_IN_FLIGHT: usize = usize::MAX;

#[derive(Clone, Copy)]
#[repr(usize)]
enum TraceEvent {
    ParentStarted = 1,
    BeforeCreateP3First,
    AfterCreateP3First,
    BeforeCreateP3Second,
    AfterCreateP3Second,
    BeforeCreateP1First,
    P1FirstBeforeYield,
    P1FirstAfterYield,
    AfterCreateP1First,
    BeforeCreateP1Second,
    P1SecondBeforeYield,
    P1SecondAfterYield,
    AfterCreateP1Second,
    ParentExiting,
    P3FirstBeforeYield,
    P3SecondBeforeYield,
    P3FirstAfterYield,
    P3SecondAfterYield,
}

const EXPECTED_TRACE: [usize; TRACE_LENGTH] = [
    TraceEvent::ParentStarted as usize,
    TraceEvent::BeforeCreateP3First as usize,
    TraceEvent::AfterCreateP3First as usize,
    TraceEvent::BeforeCreateP3Second as usize,
    TraceEvent::AfterCreateP3Second as usize,
    TraceEvent::BeforeCreateP1First as usize,
    TraceEvent::P1FirstBeforeYield as usize,
    TraceEvent::P1FirstAfterYield as usize,
    TraceEvent::AfterCreateP1First as usize,
    TraceEvent::BeforeCreateP1Second as usize,
    TraceEvent::P1SecondBeforeYield as usize,
    TraceEvent::P1SecondAfterYield as usize,
    TraceEvent::AfterCreateP1Second as usize,
    TraceEvent::ParentExiting as usize,
    TraceEvent::P3FirstBeforeYield as usize,
    TraceEvent::P3SecondBeforeYield as usize,
    TraceEvent::P3FirstAfterYield as usize,
    TraceEvent::P3SecondAfterYield as usize,
];

#[derive(Clone, Copy)]
#[repr(usize)]
enum Child {
    P3First,
    P3Second,
    P1First,
    P1Second,
}

impl Child {
    const ALL: [Self; CHILD_COUNT] = [Self::P3First, Self::P3Second, Self::P1First, Self::P1Second];

    const fn index(self) -> usize {
        self as usize
    }

    const fn priority(self) -> Priority {
        match self {
            Self::P3First | Self::P3Second => Priority::new(3),
            Self::P1First | Self::P1Second => Priority::new(1),
        }
    }

    const fn entry(self) -> TaskEntry {
        match self {
            Self::P3First => K1Diagnostic::p3_first,
            Self::P3Second => K1Diagnostic::p3_second,
            Self::P1First => K1Diagnostic::p1_first,
            Self::P1Second => K1Diagnostic::p1_second,
        }
    }

    const fn before_create(self) -> TraceEvent {
        match self {
            Self::P3First => TraceEvent::BeforeCreateP3First,
            Self::P3Second => TraceEvent::BeforeCreateP3Second,
            Self::P1First => TraceEvent::BeforeCreateP1First,
            Self::P1Second => TraceEvent::BeforeCreateP1Second,
        }
    }

    const fn after_create(self) -> TraceEvent {
        match self {
            Self::P3First => TraceEvent::AfterCreateP3First,
            Self::P3Second => TraceEvent::AfterCreateP3Second,
            Self::P1First => TraceEvent::AfterCreateP1First,
            Self::P1Second => TraceEvent::AfterCreateP1Second,
        }
    }

    const fn before_yield(self) -> TraceEvent {
        match self {
            Self::P3First => TraceEvent::P3FirstBeforeYield,
            Self::P3Second => TraceEvent::P3SecondBeforeYield,
            Self::P1First => TraceEvent::P1FirstBeforeYield,
            Self::P1Second => TraceEvent::P1SecondBeforeYield,
        }
    }

    const fn after_yield(self) -> TraceEvent {
        match self {
            Self::P3First => TraceEvent::P3FirstAfterYield,
            Self::P3Second => TraceEvent::P3SecondAfterYield,
            Self::P1First => TraceEvent::P1FirstAfterYield,
            Self::P1Second => TraceEvent::P1SecondAfterYield,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::P3First => "p3-a",
            Self::P3Second => "p3-b",
            Self::P1First => "p1-a",
            Self::P1Second => "p1-b",
        }
    }
}

struct ChildObservations {
    entry_id: AtomicU32,
    before_id: AtomicU32,
    before_parent: AtomicU32,
    after_id: AtomicU32,
    after_parent: AtomicU32,
}

impl ChildObservations {
    const fn new() -> Self {
        Self {
            entry_id: AtomicU32::new(UNSET_ID),
            before_id: AtomicU32::new(UNSET_ID),
            before_parent: AtomicU32::new(UNSET_ID),
            after_id: AtomicU32::new(UNSET_ID),
            after_parent: AtomicU32::new(UNSET_ID),
        }
    }

    fn reset(&self) {
        self.entry_id.store(UNSET_ID, Ordering::SeqCst);
        self.before_id.store(UNSET_ID, Ordering::SeqCst);
        self.before_parent.store(UNSET_ID, Ordering::SeqCst);
        self.after_id.store(UNSET_ID, Ordering::SeqCst);
        self.after_parent.store(UNSET_ID, Ordering::SeqCst);
    }

    fn matches(&self, id: TaskId, parent: TaskId) -> bool {
        let id = id.get();
        let parent = parent.get();
        self.entry_id.load(Ordering::SeqCst) == id
            && self.before_id.load(Ordering::SeqCst) == id
            && self.before_parent.load(Ordering::SeqCst) == parent
            && self.after_id.load(Ordering::SeqCst) == id
            && self.after_parent.load(Ordering::SeqCst) == parent
    }
}

/// Shared state is reset only while the finite scheduler is idle. Atomics make
/// task-to-task observations explicit and prevent the compiler from assuming
/// that an SVC scheduling point cannot change the diagnostic state.
struct K1Diagnostic {
    trace_index: AtomicUsize,
    trace: [AtomicUsize; TRACE_LENGTH],
    trace_overflowed: AtomicBool,
    parent_entry_id: AtomicU32,
    parent_observed_id: AtomicU32,
    parent_has_parent: AtomicBool,
    child_in_flight: AtomicUsize,
    create_failed: AtomicBool,
    returned_ids: [AtomicU32; CHILD_COUNT],
    ran_before_create_returned: [AtomicBool; CHILD_COUNT],
    observations: [ChildObservations; CHILD_COUNT],
}

static K1_DIAGNOSTIC: K1Diagnostic = K1Diagnostic::new();

impl K1Diagnostic {
    const fn new() -> Self {
        Self {
            trace_index: AtomicUsize::new(0),
            trace: [const { AtomicUsize::new(0) }; TRACE_LENGTH],
            trace_overflowed: AtomicBool::new(false),
            parent_entry_id: AtomicU32::new(UNSET_ID),
            parent_observed_id: AtomicU32::new(UNSET_ID),
            parent_has_parent: AtomicBool::new(false),
            child_in_flight: AtomicUsize::new(NO_CHILD_IN_FLIGHT),
            create_failed: AtomicBool::new(false),
            returned_ids: [const { AtomicU32::new(UNSET_ID) }; CHILD_COUNT],
            ran_before_create_returned: [const { AtomicBool::new(false) }; CHILD_COUNT],
            observations: [const { ChildObservations::new() }; CHILD_COUNT],
        }
    }

    fn reset(&self) {
        self.trace_index.store(0, Ordering::SeqCst);
        self.trace_overflowed.store(false, Ordering::SeqCst);
        for event in &self.trace {
            event.store(0, Ordering::SeqCst);
        }
        self.parent_entry_id.store(UNSET_ID, Ordering::SeqCst);
        self.parent_observed_id.store(UNSET_ID, Ordering::SeqCst);
        self.parent_has_parent.store(false, Ordering::SeqCst);
        self.child_in_flight
            .store(NO_CHILD_IN_FLIGHT, Ordering::SeqCst);
        self.create_failed.store(false, Ordering::SeqCst);
        for id in &self.returned_ids {
            id.store(UNSET_ID, Ordering::SeqCst);
        }
        for ran_early in &self.ran_before_create_returned {
            ran_early.store(false, Ordering::SeqCst);
        }
        for observation in &self.observations {
            observation.reset();
        }
    }

    fn record(&self, event: TraceEvent) {
        let index = self.trace_index.fetch_add(1, Ordering::SeqCst);
        if let Some(slot) = self.trace.get(index) {
            slot.store(event as usize, Ordering::SeqCst);
        } else {
            self.trace_overflowed.store(true, Ordering::SeqCst);
        }
    }

    extern "C" fn parent(entry_id: TaskId) -> ! {
        K1_DIAGNOSTIC.run_parent(entry_id)
    }

    extern "C" fn p3_first(entry_id: TaskId) -> ! {
        K1_DIAGNOSTIC.run_child(Child::P3First, entry_id)
    }

    extern "C" fn p3_second(entry_id: TaskId) -> ! {
        K1_DIAGNOSTIC.run_child(Child::P3Second, entry_id)
    }

    extern "C" fn p1_first(entry_id: TaskId) -> ! {
        K1_DIAGNOSTIC.run_child(Child::P1First, entry_id)
    }

    extern "C" fn p1_second(entry_id: TaskId) -> ! {
        K1_DIAGNOSTIC.run_child(Child::P1Second, entry_id)
    }

    fn run_parent(&self, entry_id: TaskId) -> ! {
        let observed_id = CurrentTask::id();
        let parent = CurrentTask::parent_id();
        self.parent_entry_id.store(entry_id.get(), Ordering::SeqCst);
        self.parent_observed_id
            .store(observed_id.get(), Ordering::SeqCst);
        self.parent_has_parent
            .store(parent.is_some(), Ordering::SeqCst);
        self.record(TraceEvent::ParentStarted);

        for child in Child::ALL {
            self.record(child.before_create());
            self.child_in_flight.store(child.index(), Ordering::SeqCst);
            let result = CurrentTask::create(child.priority(), child.entry());
            self.child_in_flight
                .store(NO_CHILD_IN_FLIGHT, Ordering::SeqCst);

            match result {
                Ok(id) => {
                    self.returned_ids[child.index()].store(id.get(), Ordering::SeqCst);
                    let mut logger = KERNEL.logger();
                    logger
                        .debug(format_args!(
                            "k1 parent: Create({}, {}) returned tid {id}",
                            child.priority().get(),
                            child.name()
                        ))
                        .ok();
                }
                Err(error) => {
                    self.create_failed.store(true, Ordering::SeqCst);
                    let mut logger = KERNEL.logger();
                    logger
                        .error(format_args!(
                            "k1 parent: Create({}, {}) failed: {error:?}",
                            child.priority().get(),
                            child.name()
                        ))
                        .ok();
                }
            }
            self.record(child.after_create());
        }

        self.record(TraceEvent::ParentExiting);
        CurrentTask::exit();
    }

    fn run_child(&self, child: Child, entry_id: TaskId) -> ! {
        let index = child.index();
        let observation = &self.observations[index];
        observation.entry_id.store(entry_id.get(), Ordering::SeqCst);

        if self.child_in_flight.load(Ordering::SeqCst) == index
            && self.returned_ids[index].load(Ordering::SeqCst) == UNSET_ID
        {
            self.ran_before_create_returned[index].store(true, Ordering::SeqCst);
        }

        let before_id = CurrentTask::id();
        let before_parent = CurrentTask::parent_id();
        observation
            .before_id
            .store(before_id.get(), Ordering::SeqCst);
        observation.before_parent.store(
            before_parent.map_or(UNSET_ID, TaskId::get),
            Ordering::SeqCst,
        );
        self.record(child.before_yield());
        Self::log_observation(child, "before", before_id, before_parent);

        CurrentTask::yield_now();

        let after_id = CurrentTask::id();
        let after_parent = CurrentTask::parent_id();
        observation.after_id.store(after_id.get(), Ordering::SeqCst);
        observation
            .after_parent
            .store(after_parent.map_or(UNSET_ID, TaskId::get), Ordering::SeqCst);
        self.record(child.after_yield());
        Self::log_observation(child, "after", after_id, after_parent);
        CurrentTask::exit();
    }

    fn log_observation(child: Child, phase: &'static str, id: TaskId, parent: Option<TaskId>) {
        let mut logger = KERNEL.logger();
        match parent {
            Some(parent) => logger
                .debug(format_args!(
                    "k1 child {}: {phase} yield (tid {id}, parent {parent})",
                    child.name()
                ))
                .ok(),
            None => logger
                .debug(format_args!(
                    "k1 child {}: {phase} yield (tid {id}, no parent)",
                    child.name()
                ))
                .ok(),
        };
    }

    fn verify(&self, root_id: TaskId, outcome: RunOutcome) -> Verification {
        let trace_matches = !self.trace_overflowed.load(Ordering::SeqCst)
            && self.trace_index.load(Ordering::SeqCst) == TRACE_LENGTH
            && self
                .trace
                .iter()
                .zip(EXPECTED_TRACE)
                .all(|(actual, expected)| actual.load(Ordering::SeqCst) == expected);

        let parent_matches = self.parent_entry_id.load(Ordering::SeqCst) == root_id.get()
            && self.parent_observed_id.load(Ordering::SeqCst) == root_id.get()
            && !self.parent_has_parent.load(Ordering::SeqCst);

        let child_identities_match = Child::ALL.into_iter().all(|child| {
            let raw_id = self.returned_ids[child.index()].load(Ordering::SeqCst);
            raw_id != UNSET_ID
                && self.observations[child.index()].matches(TaskId::new(raw_id), root_id)
        });

        let higher_priority_preempted = [Child::P1First, Child::P1Second]
            .into_iter()
            .all(|child| self.ran_before_create_returned[child.index()].load(Ordering::SeqCst));
        let lower_priority_waited = [Child::P3First, Child::P3Second]
            .into_iter()
            .all(|child| !self.ran_before_create_returned[child.index()].load(Ordering::SeqCst));

        Verification {
            trace_matches,
            parent_matches,
            child_identities_match,
            priority_behavior_matches: higher_priority_preempted && lower_priority_waited,
            all_tasks_exited: outcome.exited_tasks() == TASK_COUNT,
            create_succeeded: !self.create_failed.load(Ordering::SeqCst),
        }
    }
}

struct Verification {
    trace_matches: bool,
    parent_matches: bool,
    child_identities_match: bool,
    priority_behavior_matches: bool,
    all_tasks_exited: bool,
    create_succeeded: bool,
}

impl Verification {
    const fn passed(&self) -> bool {
        self.trace_matches
            && self.parent_matches
            && self.child_identities_match
            && self.priority_behavior_matches
            && self.all_tasks_exited
            && self.create_succeeded
    }
}

pub(crate) struct K1;

impl K1 {
    pub(crate) const fn command() -> Command<Raspi4> {
        Command::new("k1", Self::run)
    }

    fn run(kernel: &Kernel<Raspi4>, _arguments: &str) -> CommandResult {
        K1_DIAGNOSTIC.reset();
        let mut logger = kernel.logger();
        logger.info("trains-rs: K1 task lifecycle diagnostic").ok();

        let root_id = match kernel.create_task(K1Diagnostic::parent, Priority::new(2)) {
            Ok(id) => id,
            Err(error) => {
                logger
                    .error(format_args!(
                        "trains-rs: K1 root creation failed: {error:?}"
                    ))
                    .ok();
                return CommandResult::Failure;
            }
        };

        let outcome = match kernel.run_tasks() {
            Ok(outcome) => outcome,
            Err(error) => {
                logger
                    .error(format_args!("trains-rs: K1 finite run failed: {error:?}"))
                    .ok();
                return CommandResult::Failure;
            }
        };
        let verification = K1_DIAGNOSTIC.verify(root_id, outcome);

        if verification.passed() {
            logger
                .info("trains-rs: K1 passed (priority, FIFO, identity, exit, finite return)")
                .ok();
            CommandResult::Success
        } else {
            logger
                .error(format_args!(
                    "trains-rs: K1 failed (trace={}, parent={}, children={}, priority={}, exits={}, creates={})",
                    verification.trace_matches,
                    verification.parent_matches,
                    verification.child_identities_match,
                    verification.priority_behavior_matches,
                    verification.all_tasks_exited,
                    verification.create_succeeded,
                ))
                .ok();
            CommandResult::Failure
        }
    }
}

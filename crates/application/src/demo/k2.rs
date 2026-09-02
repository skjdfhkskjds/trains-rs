//! End-to-end K2 synchronous IPC and name-service diagnostic.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use trains_kernel::{
    CurrentTask, IpcError, Kernel, Priority, TaskEntry, TaskId,
    runtime::{Command, CommandResult},
};
use trains_platform::Raspi4;

use crate::services::name_server::{NameServer, NameServerError, NameServerHandle};

const UNSET_ID: u32 = u32::MAX;
const INVALID_TARGET: TaskId = TaskId::new(u32::MAX);

static FAILURES: AtomicUsize = AtomicUsize::new(0);

fn check(condition: bool) {
    if !condition {
        FAILURES.fetch_add(1, Ordering::SeqCst);
    }
}

fn load_id(id: &AtomicU32) -> TaskId {
    TaskId::new(id.load(Ordering::SeqCst))
}

fn reset_ids(ids: &[AtomicU32]) {
    for id in ids {
        id.store(UNSET_ID, Ordering::SeqCst);
    }
}

fn create_root(kernel: &Kernel<Raspi4>, entry: TaskEntry, priority: Priority) -> Option<TaskId> {
    match kernel.create_task(entry, priority) {
        Ok(id) => Some(id),
        Err(_) => {
            check(false);
            None
        }
    }
}

fn finish_run(kernel: &Kernel<Raspi4>, expected_exits: usize, failures_before: usize) -> bool {
    let outcome_matches = kernel
        .run_tasks()
        .is_ok_and(|outcome| outcome.exited_tasks() == expected_exits);
    outcome_matches && FAILURES.load(Ordering::SeqCst) == failures_before
}

fn drain_failed_setup(kernel: &Kernel<Raspi4>, roots_created: usize) {
    if roots_created != 0 {
        // A partial workload either exits or is classified as a finite-run
        // deadlock, which clears its slots for the next repeatable command.
        let _ = kernel.run_tasks();
    }
}

// -------------------------------------------------------------------------
// Receive-first rendezvous, logical lengths, and typed IPC errors.
// -------------------------------------------------------------------------

const RECEIVE_FIRST_ROOTS: usize = 2;
const RECEIVE_FIRST_EXITS: usize = RECEIVE_FIRST_ROOTS + 1;
const RECEIVE_FIRST_MESSAGE: &[u8] = b"message-is-long";
const RECEIVE_FIRST_REPLY: &[u8] = b"reply-is-long";

static RECEIVE_FIRST_RECEIVER: AtomicU32 = AtomicU32::new(UNSET_ID);
static RECEIVE_FIRST_SENDER: AtomicU32 = AtomicU32::new(UNSET_ID);
static RECEIVE_FIRST_WAITING: AtomicBool = AtomicBool::new(false);
static RECEIVE_FIRST_SENDER_PHASE: AtomicUsize = AtomicUsize::new(0);
static RECEIVE_FIRST_INTRUDER_DONE: AtomicBool = AtomicBool::new(false);

fn reset_receive_first() {
    RECEIVE_FIRST_RECEIVER.store(UNSET_ID, Ordering::SeqCst);
    RECEIVE_FIRST_SENDER.store(UNSET_ID, Ordering::SeqCst);
    RECEIVE_FIRST_WAITING.store(false, Ordering::SeqCst);
    RECEIVE_FIRST_SENDER_PHASE.store(0, Ordering::SeqCst);
    RECEIVE_FIRST_INTRUDER_DONE.store(false, Ordering::SeqCst);
}

extern "C" fn receive_first_receiver(_entry_id: TaskId) -> ! {
    let mut message = [0; 4];
    RECEIVE_FIRST_WAITING.store(true, Ordering::SeqCst);
    let received = CurrentTask::receive(&mut message);
    let expected_sender = load_id(&RECEIVE_FIRST_SENDER);

    match received {
        Ok((sender, logical_length)) => {
            check(sender == expected_sender);
            check(logical_length == RECEIVE_FIRST_MESSAGE.len());
            check(message == RECEIVE_FIRST_MESSAGE[..message.len()]);
        }
        Err(_) => check(false),
    }
    check(RECEIVE_FIRST_SENDER_PHASE.load(Ordering::SeqCst) == 1);

    let intruder = CurrentTask::create(Priority::HIGHEST, receive_first_intruder);
    check(intruder.is_ok());
    check(RECEIVE_FIRST_INTRUDER_DONE.load(Ordering::SeqCst));
    check(RECEIVE_FIRST_SENDER_PHASE.load(Ordering::SeqCst) == 1);

    check(
        CurrentTask::reply(expected_sender, RECEIVE_FIRST_REPLY) == Ok(RECEIVE_FIRST_REPLY.len()),
    );
    // The receiver has higher priority than the newly readied sender, so this
    // second call observes a valid task which is no longer reply-blocked.
    check(CurrentTask::reply(expected_sender, b"duplicate") == Err(IpcError::NotReplyBlocked));
    CurrentTask::exit()
}

extern "C" fn receive_first_intruder(_entry_id: TaskId) -> ! {
    let sender = load_id(&RECEIVE_FIRST_SENDER);
    check(CurrentTask::reply(sender, b"unauthorized") == Err(IpcError::NotMessageReceiver));
    RECEIVE_FIRST_INTRUDER_DONE.store(true, Ordering::SeqCst);
    CurrentTask::exit()
}

extern "C" fn receive_first_sender(_entry_id: TaskId) -> ! {
    check(RECEIVE_FIRST_WAITING.load(Ordering::SeqCst));
    check(CurrentTask::send(INVALID_TARGET, b"invalid", &mut []) == Err(IpcError::TaskNotFound));

    RECEIVE_FIRST_SENDER_PHASE.store(1, Ordering::SeqCst);
    let mut reply = [0; 3];
    let result = CurrentTask::send(
        load_id(&RECEIVE_FIRST_RECEIVER),
        RECEIVE_FIRST_MESSAGE,
        &mut reply,
    );
    RECEIVE_FIRST_SENDER_PHASE.store(2, Ordering::SeqCst);
    check(result == Ok(RECEIVE_FIRST_REPLY.len()));
    check(reply == RECEIVE_FIRST_REPLY[..reply.len()]);
    check(RECEIVE_FIRST_INTRUDER_DONE.load(Ordering::SeqCst));
    CurrentTask::exit()
}

fn run_receive_first(kernel: &Kernel<Raspi4>) -> bool {
    reset_receive_first();
    let failures_before = FAILURES.load(Ordering::SeqCst);
    let mut roots_created = 0;

    let receiver = create_root(kernel, receive_first_receiver, Priority::new(1));
    roots_created += usize::from(receiver.is_some());
    let sender = create_root(kernel, receive_first_sender, Priority::new(2));
    roots_created += usize::from(sender.is_some());
    let (Some(receiver), Some(sender)) = (receiver, sender) else {
        drain_failed_setup(kernel, roots_created);
        return false;
    };
    RECEIVE_FIRST_RECEIVER.store(receiver.get(), Ordering::SeqCst);
    RECEIVE_FIRST_SENDER.store(sender.get(), Ordering::SeqCst);

    finish_run(kernel, RECEIVE_FIRST_EXITS, failures_before)
        && RECEIVE_FIRST_SENDER_PHASE.load(Ordering::SeqCst) == 2
}

// -------------------------------------------------------------------------
// Send-first FIFO plus a modest repeated rendezvous workload.
// -------------------------------------------------------------------------

const FIFO_SENDERS: usize = 3;
const ROUND_TRIPS: usize = 32;
const SEND_FIRST_EXITS: usize = FIFO_SENDERS + 2;

static SEND_FIRST_RECEIVER: AtomicU32 = AtomicU32::new(UNSET_ID);
static FIFO_SENDER_IDS: [AtomicU32; FIFO_SENDERS] =
    [const { AtomicU32::new(UNSET_ID) }; FIFO_SENDERS];
static FIFO_SENDER_PHASES: [AtomicUsize; FIFO_SENDERS] =
    [const { AtomicUsize::new(0) }; FIFO_SENDERS];
static FIFO_RECEIVED: AtomicUsize = AtomicUsize::new(0);
static ROUND_TRIPS_COMPLETED: AtomicUsize = AtomicUsize::new(0);

fn reset_send_first() {
    SEND_FIRST_RECEIVER.store(UNSET_ID, Ordering::SeqCst);
    reset_ids(&FIFO_SENDER_IDS);
    for phase in &FIFO_SENDER_PHASES {
        phase.store(0, Ordering::SeqCst);
    }
    FIFO_RECEIVED.store(0, Ordering::SeqCst);
    ROUND_TRIPS_COMPLETED.store(0, Ordering::SeqCst);
}

extern "C" fn fifo_sender_zero(entry_id: TaskId) -> ! {
    run_fifo_sender(0, entry_id)
}

extern "C" fn fifo_sender_one(entry_id: TaskId) -> ! {
    run_fifo_sender(1, entry_id)
}

extern "C" fn fifo_sender_two(entry_id: TaskId) -> ! {
    run_fifo_sender(2, entry_id)
}

fn run_fifo_sender(index: usize, entry_id: TaskId) -> ! {
    check(entry_id == load_id(&FIFO_SENDER_IDS[index]));
    FIFO_SENDER_PHASES[index].store(1, Ordering::SeqCst);
    let message = [0xf0, index as u8, 0x55, 0xaa];
    let mut reply = [0; 2];
    let result = CurrentTask::send(load_id(&SEND_FIRST_RECEIVER), &message, &mut reply);
    FIFO_SENDER_PHASES[index].store(2, Ordering::SeqCst);
    check(result == Ok(reply.len()));
    check(reply == [0xac, index as u8]);
    CurrentTask::exit()
}

extern "C" fn repeated_sender(_entry_id: TaskId) -> ! {
    let mut message = [0xb0; 64];
    let mut reply = [0; 64];
    for round in 0..ROUND_TRIPS {
        message[1] = round as u8;
        reply.fill(0);
        let result = CurrentTask::send(load_id(&SEND_FIRST_RECEIVER), &message, &mut reply);
        check(result == Ok(reply.len()));
        check(reply[0] == 0xb1 && reply[1] == round as u8);
        ROUND_TRIPS_COMPLETED.fetch_add(1, Ordering::SeqCst);
    }
    CurrentTask::exit()
}

extern "C" fn send_first_receiver(_entry_id: TaskId) -> ! {
    for index in 0..FIFO_SENDERS {
        let mut message = [0; 4];
        match CurrentTask::receive(&mut message) {
            Ok((sender, logical_length)) => {
                check(sender == load_id(&FIFO_SENDER_IDS[index]));
                check(logical_length == message.len());
                check(message == [0xf0, index as u8, 0x55, 0xaa]);
                check(FIFO_SENDER_PHASES[index].load(Ordering::SeqCst) == 1);
                check(CurrentTask::reply(sender, &[0xac, index as u8]) == Ok(2));
            }
            Err(_) => check(false),
        }
        FIFO_RECEIVED.fetch_add(1, Ordering::SeqCst);
    }

    for round in 0..ROUND_TRIPS {
        let mut message = [0; 64];
        match CurrentTask::receive(&mut message) {
            Ok((sender, logical_length)) => {
                check(logical_length == message.len());
                check(message[0] == 0xb0 && message[1] == round as u8);
                let mut reply = [0xb1; 64];
                reply[1] = round as u8;
                check(CurrentTask::reply(sender, &reply) == Ok(reply.len()));
            }
            Err(_) => check(false),
        }
    }
    CurrentTask::exit()
}

fn run_send_first(kernel: &Kernel<Raspi4>) -> bool {
    reset_send_first();
    let failures_before = FAILURES.load(Ordering::SeqCst);
    let mut roots_created = 0;

    let receiver = create_root(kernel, send_first_receiver, Priority::new(3));
    roots_created += usize::from(receiver.is_some());
    let entries = [fifo_sender_zero, fifo_sender_one, fifo_sender_two];
    let mut senders_created = true;
    for (index, entry) in entries.into_iter().enumerate() {
        let sender = create_root(kernel, entry, Priority::new(1));
        roots_created += usize::from(sender.is_some());
        if let Some(sender) = sender {
            FIFO_SENDER_IDS[index].store(sender.get(), Ordering::SeqCst);
        } else {
            senders_created = false;
        }
    }
    let repeated = create_root(kernel, repeated_sender, Priority::new(2));
    roots_created += usize::from(repeated.is_some());

    let (Some(receiver), true, Some(_)) = (receiver, senders_created, repeated) else {
        drain_failed_setup(kernel, roots_created);
        return false;
    };
    SEND_FIRST_RECEIVER.store(receiver.get(), Ordering::SeqCst);

    finish_run(kernel, SEND_FIRST_EXITS, failures_before)
        && FIFO_RECEIVED.load(Ordering::SeqCst) == FIFO_SENDERS
        && FIFO_SENDER_PHASES
            .iter()
            .all(|phase| phase.load(Ordering::SeqCst) == 2)
        && ROUND_TRIPS_COMPLETED.load(Ordering::SeqCst) == ROUND_TRIPS
}

// -------------------------------------------------------------------------
// Full incoming-sender queue and recovery.
// -------------------------------------------------------------------------

const WAITING_SENDER_CAPACITY: usize = 10;
const SATURATION_SENDERS: usize = WAITING_SENDER_CAPACITY + 1;
const SATURATION_EXITS: usize = SATURATION_SENDERS + 1;

static SATURATION_RECEIVER: AtomicU32 = AtomicU32::new(UNSET_ID);
static SATURATION_SENDER_IDS: [AtomicU32; SATURATION_SENDERS] =
    [const { AtomicU32::new(UNSET_ID) }; SATURATION_SENDERS];
static SATURATION_REPLIED: AtomicUsize = AtomicUsize::new(0);
static SATURATION_QUEUE_FULL: AtomicUsize = AtomicUsize::new(0);
static SATURATION_RECEIVED: AtomicUsize = AtomicUsize::new(0);

fn reset_saturation() {
    SATURATION_RECEIVER.store(UNSET_ID, Ordering::SeqCst);
    reset_ids(&SATURATION_SENDER_IDS);
    SATURATION_REPLIED.store(0, Ordering::SeqCst);
    SATURATION_QUEUE_FULL.store(0, Ordering::SeqCst);
    SATURATION_RECEIVED.store(0, Ordering::SeqCst);
}

extern "C" fn saturation_sender(entry_id: TaskId) -> ! {
    let message = entry_id.get().to_le_bytes();
    let mut reply = [0; 1];
    match CurrentTask::send(load_id(&SATURATION_RECEIVER), &message, &mut reply) {
        Ok(length) => {
            check(length == reply.len() && reply[0] == 0xcc);
            SATURATION_REPLIED.fetch_add(1, Ordering::SeqCst);
        }
        Err(IpcError::QueueFull) => {
            check(entry_id == load_id(&SATURATION_SENDER_IDS[WAITING_SENDER_CAPACITY]));
            SATURATION_QUEUE_FULL.fetch_add(1, Ordering::SeqCst);
        }
        Err(_) => check(false),
    }
    CurrentTask::exit()
}

extern "C" fn saturation_receiver(_entry_id: TaskId) -> ! {
    for expected_id in SATURATION_SENDER_IDS.iter().take(WAITING_SENDER_CAPACITY) {
        let mut message = [0; size_of::<u32>()];
        match CurrentTask::receive(&mut message) {
            Ok((sender, logical_length)) => {
                let expected = load_id(expected_id);
                check(sender == expected);
                check(logical_length == message.len());
                check(u32::from_le_bytes(message) == expected.get());
                check(CurrentTask::reply(sender, &[0xcc]) == Ok(1));
            }
            Err(_) => check(false),
        }
        SATURATION_RECEIVED.fetch_add(1, Ordering::SeqCst);
    }
    CurrentTask::exit()
}

fn run_saturation(kernel: &Kernel<Raspi4>) -> bool {
    reset_saturation();
    let failures_before = FAILURES.load(Ordering::SeqCst);
    let mut roots_created = 0;

    let receiver = create_root(kernel, saturation_receiver, Priority::new(3));
    roots_created += usize::from(receiver.is_some());
    let mut senders_created = true;
    for expected_id in &SATURATION_SENDER_IDS {
        let sender = create_root(kernel, saturation_sender, Priority::new(1));
        roots_created += usize::from(sender.is_some());
        if let Some(sender) = sender {
            expected_id.store(sender.get(), Ordering::SeqCst);
        } else {
            senders_created = false;
        }
    }
    let (Some(receiver), true) = (receiver, senders_created) else {
        drain_failed_setup(kernel, roots_created);
        return false;
    };
    SATURATION_RECEIVER.store(receiver.get(), Ordering::SeqCst);

    finish_run(kernel, SATURATION_EXITS, failures_before)
        && SATURATION_REPLIED.load(Ordering::SeqCst) == WAITING_SENDER_CAPACITY
        && SATURATION_QUEUE_FULL.load(Ordering::SeqCst) == 1
        && SATURATION_RECEIVED.load(Ordering::SeqCst) == WAITING_SENDER_CAPACITY
}

// -------------------------------------------------------------------------
// Bounded name server and a two-client delayed-reply RPS round.
// -------------------------------------------------------------------------

const SERVICE_REQUESTS: usize = 7;
const SERVICE_EXITS: usize = 6;
const SIGN_UP: u8 = 1;
const PLAY_ROCK: u8 = 2;
const PLAY_SCISSORS: u8 = 3;
const SIGNED_UP: u8 = 0x10;
const YOU_WIN: u8 = 0x11;
const YOU_LOSE: u8 = 0x12;

static NAME_SERVER_ID: AtomicU32 = AtomicU32::new(UNSET_ID);
static RPS_SERVER_ID: AtomicU32 = AtomicU32::new(UNSET_ID);
static RPS_CLIENT_A_ID: AtomicU32 = AtomicU32::new(UNSET_ID);
static RPS_CLIENT_B_ID: AtomicU32 = AtomicU32::new(UNSET_ID);
static NAME_CHECKS_DONE: AtomicBool = AtomicBool::new(false);
static RPS_REGISTERED: AtomicBool = AtomicBool::new(false);
static RPS_CLIENT_A_PHASE: AtomicUsize = AtomicUsize::new(0);
static RPS_CLIENT_B_PHASE: AtomicUsize = AtomicUsize::new(0);
static RPS_SERVER_COMPLETED: AtomicBool = AtomicBool::new(false);

fn reset_services() {
    NAME_SERVER_ID.store(UNSET_ID, Ordering::SeqCst);
    RPS_SERVER_ID.store(UNSET_ID, Ordering::SeqCst);
    RPS_CLIENT_A_ID.store(UNSET_ID, Ordering::SeqCst);
    RPS_CLIENT_B_ID.store(UNSET_ID, Ordering::SeqCst);
    NAME_CHECKS_DONE.store(false, Ordering::SeqCst);
    RPS_REGISTERED.store(false, Ordering::SeqCst);
    RPS_CLIENT_A_PHASE.store(0, Ordering::SeqCst);
    RPS_CLIENT_B_PHASE.store(0, Ordering::SeqCst);
    RPS_SERVER_COMPLETED.store(false, Ordering::SeqCst);
}

fn name_server_handle() -> NameServerHandle {
    NameServerHandle::new(load_id(&NAME_SERVER_ID))
}

extern "C" fn name_first(entry_id: TaskId) -> ! {
    check(
        name_server_handle().register_as(b"replace-me") == Ok(()) && CurrentTask::id() == entry_id,
    );
    CurrentTask::exit()
}

extern "C" fn name_second(entry_id: TaskId) -> ! {
    // The first registration has been processed before its synchronous Send
    // is replied to, even if this equal-priority task runs before the first
    // client resumes from that reply.
    let handle = name_server_handle();
    check(handle.who_is(b"unknown") == Err(NameServerError::NotFound));
    check(handle.register_as(b"replace-me") == Ok(()));
    check(handle.who_is(b"replace-me") == Ok(entry_id));
    NAME_CHECKS_DONE.store(true, Ordering::SeqCst);
    CurrentTask::exit()
}

extern "C" fn rps_server(entry_id: TaskId) -> ! {
    check(name_server_handle().register_as(b"rps") == Ok(()));
    check(CurrentTask::id() == entry_id);
    RPS_REGISTERED.store(true, Ordering::SeqCst);

    let mut request = [0; 1];
    let first_signup = CurrentTask::receive(&mut request);
    let first_sender = first_signup.map_or(INVALID_TARGET, |(sender, length)| {
        check(length == 1 && request[0] == SIGN_UP);
        sender
    });
    check(first_sender == load_id(&RPS_CLIENT_A_ID));
    check(RPS_CLIENT_A_PHASE.load(Ordering::SeqCst) == 1);

    let second_signup = CurrentTask::receive(&mut request);
    let second_sender = second_signup.map_or(INVALID_TARGET, |(sender, length)| {
        check(length == 1 && request[0] == SIGN_UP);
        sender
    });
    check(second_sender == load_id(&RPS_CLIENT_B_ID));
    check(RPS_CLIENT_A_PHASE.load(Ordering::SeqCst) == 1);
    check(CurrentTask::reply(first_sender, &[SIGNED_UP]) == Ok(1));
    check(CurrentTask::reply(second_sender, &[SIGNED_UP]) == Ok(1));

    let first_play = CurrentTask::receive(&mut request);
    let first_player = first_play.map_or(INVALID_TARGET, |(sender, length)| {
        check(length == 1 && request[0] == PLAY_ROCK);
        sender
    });
    check(first_player == load_id(&RPS_CLIENT_A_ID));
    check(RPS_CLIENT_A_PHASE.load(Ordering::SeqCst) == 3);

    let second_play = CurrentTask::receive(&mut request);
    let second_player = second_play.map_or(INVALID_TARGET, |(sender, length)| {
        check(length == 1 && request[0] == PLAY_SCISSORS);
        sender
    });
    check(second_player == load_id(&RPS_CLIENT_B_ID));
    check(RPS_CLIENT_A_PHASE.load(Ordering::SeqCst) == 3);
    check(CurrentTask::reply(first_player, &[YOU_WIN]) == Ok(1));
    check(CurrentTask::reply(second_player, &[YOU_LOSE]) == Ok(1));
    RPS_SERVER_COMPLETED.store(true, Ordering::SeqCst);
    CurrentTask::exit()
}

extern "C" fn rps_client_a(_entry_id: TaskId) -> ! {
    run_rps_client(true)
}

extern "C" fn rps_client_b(_entry_id: TaskId) -> ! {
    run_rps_client(false)
}

fn run_rps_client(first: bool) -> ! {
    check(NAME_CHECKS_DONE.load(Ordering::SeqCst));
    let server = match name_server_handle().who_is(b"rps") {
        Ok(server) => server,
        Err(_) => {
            check(false);
            load_id(&RPS_SERVER_ID)
        }
    };
    check(server == load_id(&RPS_SERVER_ID));

    let phase = if first {
        &RPS_CLIENT_A_PHASE
    } else {
        &RPS_CLIENT_B_PHASE
    };
    if !first {
        check(RPS_CLIENT_A_PHASE.load(Ordering::SeqCst) == 1);
    }
    phase.store(1, Ordering::SeqCst);
    let mut reply = [0; 1];
    check(CurrentTask::send(server, &[SIGN_UP], &mut reply) == Ok(1));
    check(reply[0] == SIGNED_UP);
    phase.store(2, Ordering::SeqCst);

    if !first {
        check(RPS_CLIENT_A_PHASE.load(Ordering::SeqCst) == 3);
    }
    phase.store(3, Ordering::SeqCst);
    reply[0] = 0;
    let play = if first { PLAY_ROCK } else { PLAY_SCISSORS };
    check(CurrentTask::send(server, &[play], &mut reply) == Ok(1));
    check(reply[0] == if first { YOU_WIN } else { YOU_LOSE });
    phase.store(4, Ordering::SeqCst);
    CurrentTask::exit()
}

fn run_services(kernel: &Kernel<Raspi4>) -> bool {
    reset_services();
    let failures_before = FAILURES.load(Ordering::SeqCst);
    let mut roots_created = 0;

    let name_server = create_root(
        kernel,
        NameServer::bounded_task_entry(SERVICE_REQUESTS),
        Priority::HIGHEST,
    );
    roots_created += usize::from(name_server.is_some());
    let rps = create_root(kernel, rps_server, Priority::new(1));
    roots_created += usize::from(rps.is_some());
    let name_a = create_root(kernel, name_first, Priority::new(2));
    roots_created += usize::from(name_a.is_some());
    let name_b = create_root(kernel, name_second, Priority::new(2));
    roots_created += usize::from(name_b.is_some());
    let client_a = create_root(kernel, rps_client_a, Priority::new(3));
    roots_created += usize::from(client_a.is_some());
    let client_b = create_root(kernel, rps_client_b, Priority::new(3));
    roots_created += usize::from(client_b.is_some());

    let (Some(name_server), Some(rps), Some(_), Some(_), Some(client_a), Some(client_b)) =
        (name_server, rps, name_a, name_b, client_a, client_b)
    else {
        drain_failed_setup(kernel, roots_created);
        return false;
    };
    NAME_SERVER_ID.store(name_server.get(), Ordering::SeqCst);
    RPS_SERVER_ID.store(rps.get(), Ordering::SeqCst);
    RPS_CLIENT_A_ID.store(client_a.get(), Ordering::SeqCst);
    RPS_CLIENT_B_ID.store(client_b.get(), Ordering::SeqCst);
    check(name_server_handle().task_id() == name_server);

    finish_run(kernel, SERVICE_EXITS, failures_before)
        && NAME_CHECKS_DONE.load(Ordering::SeqCst)
        && RPS_REGISTERED.load(Ordering::SeqCst)
        && RPS_SERVER_COMPLETED.load(Ordering::SeqCst)
        && RPS_CLIENT_A_PHASE.load(Ordering::SeqCst) == 4
        && RPS_CLIENT_B_PHASE.load(Ordering::SeqCst) == 4
}

pub(crate) struct K2;

impl K2 {
    pub(crate) const fn command() -> Command<Raspi4> {
        Command::new("k2", Self::run)
    }

    fn run(kernel: &Kernel<Raspi4>, _arguments: &str) -> CommandResult {
        FAILURES.store(0, Ordering::SeqCst);
        let mut logger = kernel.logger();
        logger.info("trains-rs: K2 synchronous IPC diagnostic").ok();

        let receive_first = run_receive_first(kernel);
        logger
            .info(format_args!(
                "trains-rs: K2 receive-first/truncation/errors: {}",
                if receive_first { "passed" } else { "failed" }
            ))
            .ok();

        let send_first = run_send_first(kernel);
        logger
            .info(format_args!(
                "trains-rs: K2 send-first/FIFO/{ROUND_TRIPS} round trips: {}",
                if send_first { "passed" } else { "failed" }
            ))
            .ok();

        let saturation = run_saturation(kernel);
        logger
            .info(format_args!(
                "trains-rs: K2 bounded sender queue: {}",
                if saturation { "passed" } else { "failed" }
            ))
            .ok();

        let services = run_services(kernel);
        logger
            .info(format_args!(
                "trains-rs: K2 name service/delayed RPS: {}",
                if services { "passed" } else { "failed" }
            ))
            .ok();

        if receive_first && send_first && saturation && services {
            logger
                .info("trains-rs: K2 passed (IPC, names, delayed replies, repeatable finite runs)")
                .ok();
            CommandResult::Success
        } else {
            logger
                .error(format_args!(
                    "trains-rs: K2 failed ({} checks failed)",
                    FAILURES.load(Ordering::SeqCst)
                ))
                .ok();
            CommandResult::Failure
        }
    }
}

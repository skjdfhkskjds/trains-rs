//! Allocation-free K2 name service.
//!
//! The name server is an ordinary user task. Clients receive a
//! [`NameServerHandle`] from bootstrap, so neither the service nor its callers
//! assume that a particular task ID belongs to the server.

use core::sync::atomic::{AtomicUsize, Ordering};

use trains_kernel::{CurrentTask, IpcError, TaskEntry, TaskId};

const REGISTER_AS: u8 = 1;
const WHO_IS: u8 = 2;

const OK: u8 = 0;
const NOT_FOUND: u8 = 1;
const REGISTRY_FULL: u8 = 2;
const INVALID_REQUEST: u8 = 3;

const NAME_CAPACITY: usize = 8;
pub(crate) const MAX_NAME_LENGTH: usize = 31;
const REQUEST_CAPACITY: usize = MAX_NAME_LENGTH + 1;
const WHO_IS_REPLY_LENGTH: usize = 1 + size_of::<u32>();
const RUN_FOREVER: usize = usize::MAX;

static BOUNDED_REQUEST_LIMIT: AtomicUsize = AtomicUsize::new(0);

/// A bootstrapped reference to the name-server task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NameServerHandle {
    task_id: TaskId,
}

impl NameServerHandle {
    pub(crate) const fn new(task_id: TaskId) -> Self {
        Self { task_id }
    }

    pub(crate) const fn task_id(self) -> TaskId {
        self.task_id
    }

    /// Registers the calling task under `name`, replacing any prior owner.
    pub(crate) fn register_as(self, name: &[u8]) -> Result<(), NameServerError> {
        validate_name(name)?;
        let mut request = [0; REQUEST_CAPACITY];
        request[0] = REGISTER_AS;
        request[1..=name.len()].copy_from_slice(name);
        let mut reply = [0; 1];
        let reply_length = CurrentTask::send(self.task_id, &request[..name.len() + 1], &mut reply)
            .map_err(NameServerError::Ipc)?;

        if reply_length != reply.len() {
            return Err(NameServerError::InvalidReply);
        }
        match reply[0] {
            OK => Ok(()),
            REGISTRY_FULL => Err(NameServerError::RegistryFull),
            INVALID_REQUEST => Err(NameServerError::InvalidName),
            _ => Err(NameServerError::InvalidReply),
        }
    }

    /// Looks up `name`, returning a typed not-found result for unknown names.
    pub(crate) fn who_is(self, name: &[u8]) -> Result<TaskId, NameServerError> {
        validate_name(name)?;
        let mut request = [0; REQUEST_CAPACITY];
        request[0] = WHO_IS;
        request[1..=name.len()].copy_from_slice(name);
        let mut reply = [0; WHO_IS_REPLY_LENGTH];
        let reply_length = CurrentTask::send(self.task_id, &request[..name.len() + 1], &mut reply)
            .map_err(NameServerError::Ipc)?;

        match (reply[0], reply_length) {
            (OK, WHO_IS_REPLY_LENGTH) => {
                let raw_id = u32::from_le_bytes([reply[1], reply[2], reply[3], reply[4]]);
                Ok(TaskId::new(raw_id))
            }
            (NOT_FOUND, 1) => Err(NameServerError::NotFound),
            (INVALID_REQUEST, 1) => Err(NameServerError::InvalidName),
            _ => Err(NameServerError::InvalidReply),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NameServerError {
    Ipc(IpcError),
    InvalidName,
    RegistryFull,
    NotFound,
    InvalidReply,
}

/// Entrypoints and configuration for the name-server task.
pub(crate) struct NameServer;

impl NameServer {
    /// The resident production service. This task intentionally never exits.
    #[allow(dead_code)]
    pub(crate) const fn task_entry() -> TaskEntry {
        Self::task
    }

    /// Configures and returns a finite service entry for boot diagnostics.
    ///
    /// Bootstrap must call this only while the finite scheduler is idle.
    pub(crate) fn bounded_task_entry(request_limit: usize) -> TaskEntry {
        BOUNDED_REQUEST_LIMIT.store(request_limit, Ordering::Release);
        Self::bounded_task
    }

    extern "C" fn task(_id: TaskId) -> ! {
        Self::serve(RUN_FOREVER)
    }

    extern "C" fn bounded_task(_id: TaskId) -> ! {
        Self::serve(BOUNDED_REQUEST_LIMIT.load(Ordering::Acquire))
    }

    fn serve(request_limit: usize) -> ! {
        let mut registry = Registry::new();
        let mut processed = 0;

        while processed < request_limit {
            let mut request = [0; REQUEST_CAPACITY];
            let (sender, logical_length) = match CurrentTask::receive(&mut request) {
                Ok(received) => received,
                Err(_) => CurrentTask::exit(),
            };
            let reply = registry.handle(sender, &request, logical_length);
            if CurrentTask::reply(sender, reply.as_slice()).is_err() {
                CurrentTask::exit();
            }
            processed += 1;
        }

        CurrentTask::exit()
    }
}

#[derive(Clone, Copy)]
struct NameEntry {
    owner: TaskId,
    length: u8,
    bytes: [u8; MAX_NAME_LENGTH],
}

impl NameEntry {
    fn new(name: &[u8], owner: TaskId) -> Self {
        let mut bytes = [0; MAX_NAME_LENGTH];
        bytes[..name.len()].copy_from_slice(name);
        Self {
            owner,
            length: name.len() as u8,
            bytes,
        }
    }

    fn matches(&self, name: &[u8]) -> bool {
        usize::from(self.length) == name.len() && self.bytes[..name.len()] == *name
    }
}

struct Registry {
    entries: [Option<NameEntry>; NAME_CAPACITY],
}

impl Registry {
    const fn new() -> Self {
        Self {
            entries: [None; NAME_CAPACITY],
        }
    }

    fn handle(
        &mut self,
        sender: TaskId,
        request: &[u8; REQUEST_CAPACITY],
        logical_length: usize,
    ) -> NameReply {
        if logical_length < 2 || logical_length > request.len() {
            return NameReply::status(INVALID_REQUEST);
        }
        let name = &request[1..logical_length];
        if validate_name(name).is_err() {
            return NameReply::status(INVALID_REQUEST);
        }

        match request[0] {
            REGISTER_AS => self.register(name, sender),
            WHO_IS => self.lookup(name),
            _ => NameReply::status(INVALID_REQUEST),
        }
    }

    fn register(&mut self, name: &[u8], owner: TaskId) -> NameReply {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .flatten()
            .find(|entry| entry.matches(name))
        {
            existing.owner = owner;
            return NameReply::status(OK);
        }

        let Some(vacant) = self.entries.iter_mut().find(|entry| entry.is_none()) else {
            return NameReply::status(REGISTRY_FULL);
        };
        *vacant = Some(NameEntry::new(name, owner));
        NameReply::status(OK)
    }

    fn lookup(&self, name: &[u8]) -> NameReply {
        self.entries
            .iter()
            .flatten()
            .find(|entry| entry.matches(name))
            .map_or_else(
                || NameReply::status(NOT_FOUND),
                |entry| NameReply::task_id(entry.owner),
            )
    }
}

struct NameReply {
    bytes: [u8; WHO_IS_REPLY_LENGTH],
    length: usize,
}

impl NameReply {
    const fn status(status: u8) -> Self {
        let mut bytes = [0; WHO_IS_REPLY_LENGTH];
        bytes[0] = status;
        Self { bytes, length: 1 }
    }

    fn task_id(task_id: TaskId) -> Self {
        let mut bytes = [0; WHO_IS_REPLY_LENGTH];
        bytes[0] = OK;
        bytes[1..].copy_from_slice(&task_id.get().to_le_bytes());
        Self {
            bytes,
            length: WHO_IS_REPLY_LENGTH,
        }
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}

fn validate_name(name: &[u8]) -> Result<(), NameServerError> {
    if name.is_empty() || name.len() > MAX_NAME_LENGTH {
        Err(NameServerError::InvalidName)
    } else {
        Ok(())
    }
}

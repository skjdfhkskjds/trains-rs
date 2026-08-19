use core::arch::asm;
use core::marker::PhantomPinned;
use core::pin::{Pin, pin};

use trains_primitives::task::{Priority, TaskId};

use crate::context::RegisterContext;

pub const STACK_SIZE: usize = 8 * 1024;

pub type TaskEntry = extern "C" fn() -> !;

#[repr(align(16))]
struct TaskStack([u8; STACK_SIZE]);

/// One suspended execution context and the stack backing it.
///
/// A task must be pinned before it is initialized because its saved stack
/// pointer refers into its own `stack` field.
pub struct Task {
    id: TaskId,
    parent: Option<TaskId>,
    priority: Priority,
    context: RegisterContext,
    stack: TaskStack,
    initialized: bool,
    _pinned: PhantomPinned,
}

impl Task {
    pub const fn new(id: TaskId, parent: Option<TaskId>, priority: Priority) -> Self {
        Self {
            id,
            parent,
            priority,
            context: RegisterContext::new(0, 0),
            stack: TaskStack([0; STACK_SIZE]),
            initialized: false,
            _pinned: PhantomPinned,
        }
    }

    pub fn initialize(self: Pin<&mut Self>, entry: TaskEntry) {
        // SAFETY: `self` is pinned, and this method never move any field. The
        // resulting context may therefore retain a pointer into `stack`.
        let task = unsafe { self.get_unchecked_mut() };
        let (_, stack_top) = task.stack_bounds();
        task.context = RegisterContext::new(entry as usize, stack_top);
        task.initialized = true;
    }

    pub const fn id(&self) -> TaskId {
        self.id
    }

    pub const fn parent(&self) -> Option<TaskId> {
        self.parent
    }

    pub const fn priority(&self) -> Priority {
        self.priority
    }

    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn context(self: Pin<&Self>) -> Option<&RegisterContext> {
        self.get_ref()
            .initialized
            .then_some(&self.get_ref().context)
    }

    pub fn context_mut(self: Pin<&mut Self>) -> Option<&mut RegisterContext> {
        // SAFETY: the returned reference permits mutation but not movement of
        // the context or its pinned owning task.
        let task = unsafe { self.get_unchecked_mut() };
        task.initialized.then_some(&mut task.context)
    }

    fn stack_bounds(&self) -> (usize, usize) {
        let bottom = self.stack.0.as_ptr() as usize;
        (bottom, bottom + STACK_SIZE)
    }
}

extern "C" fn test_entry() -> ! {
    loop {
        // SAFETY: this function is only used as a valid task entry point.
        unsafe { asm!("wfe", options(nomem, nostack, preserves_flags)) };
    }
}

pub fn self_test() -> bool {
    let id = TaskId::new(7);
    let parent = TaskId::new(3);
    let priority = Priority::new(2);
    let mut task = pin!(Task::new(id, Some(parent), priority));
    task.as_mut().initialize(test_entry);
    task.as_mut().context_mut().unwrap().set_register(0, 42);

    let task = task.as_ref();
    let (stack_bottom, stack_top) = task.stack_bounds();
    let context = task.context().unwrap();

    task.id() == id
        && task.parent() == Some(parent)
        && task.priority() == priority
        && task.is_initialized()
        && context.register(0) == 42
        && context.program_counter() == test_entry as *const () as usize as u64
        && context.stack_pointer() == stack_top as u64
        && context.stack_pointer() > stack_bottom as u64
        && context.stack_pointer() & 0xf == 0
}

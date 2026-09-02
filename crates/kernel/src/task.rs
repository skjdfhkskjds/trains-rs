//! Pinned task storage, metadata, and saved execution context.

use core::arch::asm;
use core::marker::PhantomPinned;
use core::pin::{Pin, pin};

use trains_primitives::task::{Priority, TaskId};

use crate::context::{ArgumentRegister, EntryPoint, RegisterContext, StackTop};

const STACK_SIZE: usize = 8 * 1024;

#[repr(align(16))]
struct TaskStack([u8; STACK_SIZE]);

#[derive(Clone, Copy)]
pub(crate) struct TaskDescriptor {
    id: TaskId,
    parent: Option<TaskId>,
    priority: Priority,
    entry: EntryPoint,
}

impl TaskDescriptor {
    pub(crate) const fn root(id: TaskId, priority: Priority, entry: EntryPoint) -> Self {
        Self {
            id,
            parent: None,
            priority,
            entry,
        }
    }

    pub(crate) const fn child(
        id: TaskId,
        parent: TaskId,
        priority: Priority,
        entry: EntryPoint,
    ) -> Self {
        Self {
            id,
            parent: Some(parent),
            priority,
            entry,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskState {
    Ready,
    Running,
}

/// One suspended execution context and the stack backing it.
///
/// A task must be pinned before it is initialized because its saved stack
/// pointer refers into its own `stack` field.
pub(crate) struct Task {
    descriptor: TaskDescriptor,
    state: TaskState,
    context: RegisterContext,
    stack: TaskStack,
    _pinned: PhantomPinned,
}

impl Task {
    pub(crate) const fn new(descriptor: TaskDescriptor) -> Self {
        Self {
            descriptor,
            state: TaskState::Ready,
            context: RegisterContext::empty(),
            stack: TaskStack([0; STACK_SIZE]),
            _pinned: PhantomPinned,
        }
    }

    pub(crate) fn initialize(self: Pin<&mut Self>) {
        // SAFETY: `self` is pinned, and this method never moves any field. The
        // resulting context may therefore retain a pointer into `stack`.
        let task = unsafe { self.get_unchecked_mut() };
        let stack = task.stack_bounds();
        task.context = RegisterContext::for_task(task.descriptor.entry, StackTop::new(stack.top));
        task.context
            .set_argument(ArgumentRegister::First, u64::from(task.descriptor.id));
    }

    pub(crate) const fn id(&self) -> TaskId {
        self.descriptor.id
    }

    pub(crate) const fn parent(&self) -> Option<TaskId> {
        self.descriptor.parent
    }

    pub(crate) const fn priority(&self) -> Priority {
        self.descriptor.priority
    }

    pub(crate) const fn state(&self) -> TaskState {
        self.state
    }

    pub(crate) fn set_state(self: Pin<&mut Self>, state: TaskState) {
        // SAFETY: changing a field that does not structurally pin does not move
        // the task or its embedded stack.
        unsafe { self.get_unchecked_mut() }.state = state;
    }

    pub(crate) fn context(self: Pin<&Self>) -> &RegisterContext {
        &self.get_ref().context
    }

    pub(crate) fn context_mut(self: Pin<&mut Self>) -> &mut RegisterContext {
        // SAFETY: the returned reference permits mutation but not movement of
        // the context or its pinned owning task.
        &mut unsafe { self.get_unchecked_mut() }.context
    }

    fn stack_bounds(&self) -> StackBounds {
        let bottom = self.stack.0.as_ptr() as usize;
        StackBounds {
            bottom,
            top: bottom + STACK_SIZE,
        }
    }

    extern "C" fn test_entry(_id: TaskId) -> ! {
        loop {
            // SAFETY: this function is only used as a valid task entry point.
            unsafe { asm!("wfe", options(nomem, nostack, preserves_flags)) };
        }
    }

    pub(crate) fn self_test() -> bool {
        let id = TaskId::new(7);
        let parent = TaskId::new(3);
        let priority = Priority::new(2);
        let descriptor = TaskDescriptor::child(
            id,
            parent,
            priority,
            EntryPoint::new(Self::test_entry as *const () as usize),
        );
        let mut task = pin!(Self::new(descriptor));
        task.as_mut().initialize();
        task.as_mut().context_mut().set_register(0, 42);

        let task = task.as_ref();
        let stack = task.stack_bounds();
        let context = task.context();

        task.id() == id
            && task.parent() == Some(parent)
            && task.priority() == priority
            && task.state() == TaskState::Ready
            && context.register(0) == 42
            && context.program_counter() == Self::test_entry as *const () as usize as u64
            && context.stack_pointer() == stack.top as u64
            && context.stack_pointer() > stack.bottom as u64
            && context.stack_pointer() & 0xf == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StackBounds {
    bottom: usize,
    top: usize,
}

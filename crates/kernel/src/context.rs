//! Saved AArch64 execution state and exception-frame transfers.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::exceptions::ExceptionFrame;
use crate::svc::KernelCall;

const REGISTER_COUNT: usize = 31;
const SIMD_REGISTER_COUNT: usize = 32;
const EL0T_WITH_EXCEPTIONS_MASKED: u64 = 0x3c0;
const TEST_INPUT: u64 = 0x1234_5678_9abc_def0;
const TEST_OUTPUT: u64 = 0xfedc_ba98_7654_3210;

/// Address at which a newly restored execution context begins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EntryPoint(usize);

impl EntryPoint {
    pub(crate) const fn new(address: usize) -> Self {
        Self(address)
    }
}

/// Aligned, exclusive upper bound of a task's stack allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StackTop(usize);

impl StackTop {
    pub(crate) const fn new(address: usize) -> Self {
        Self(address)
    }
}

/// AArch64 argument register used to seed a new task context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArgumentRegister {
    First,
}

impl From<ArgumentRegister> for usize {
    fn from(register: ArgumentRegister) -> Self {
        match register {
            ArgumentRegister::First => 0,
        }
    }
}

/// Register state required to suspend and resume one AArch64 execution context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RegisterContext {
    registers: [u64; REGISTER_COUNT],
    simd: [u128; SIMD_REGISTER_COUNT],
    floating_point_control: u64,
    floating_point_status: u64,
    thread_pointer: u64,
    read_only_thread_pointer: u64,
    stack_pointer: u64,
    program_counter: u64,
    processor_state: u64,
}

impl RegisterContext {
    pub(crate) const fn empty() -> Self {
        Self {
            registers: [0; REGISTER_COUNT],
            simd: [0; SIMD_REGISTER_COUNT],
            floating_point_control: 0,
            floating_point_status: 0,
            thread_pointer: 0,
            read_only_thread_pointer: 0,
            stack_pointer: 0,
            program_counter: 0,
            processor_state: 0,
        }
    }

    pub(crate) const fn for_task(entry: EntryPoint, stack_top: StackTop) -> Self {
        Self {
            stack_pointer: stack_top.0 as u64,
            program_counter: entry.0 as u64,
            processor_state: EL0T_WITH_EXCEPTIONS_MASKED,
            ..Self::empty()
        }
    }

    pub(crate) const fn register(&self, index: usize) -> u64 {
        self.registers[index]
    }

    pub(crate) fn set_register(&mut self, index: usize, value: u64) {
        self.registers[index] = value;
    }

    pub(crate) fn set_argument(&mut self, register: ArgumentRegister, value: u64) {
        self.set_register(register.into(), value);
    }

    pub(crate) const fn stack_pointer(&self) -> u64 {
        self.stack_pointer
    }

    pub(crate) const fn program_counter(&self) -> u64 {
        self.program_counter
    }

    pub(crate) const fn processor_state(&self) -> u64 {
        self.processor_state
    }

    fn capture(frame: &ExceptionFrame) -> Self {
        Self {
            registers: frame.registers,
            simd: frame.simd,
            floating_point_control: frame.floating_point_control,
            floating_point_status: frame.floating_point_status,
            thread_pointer: frame.thread_pointer,
            read_only_thread_pointer: frame.read_only_thread_pointer,
            stack_pointer: frame.stack_pointer,
            program_counter: frame.elr,
            processor_state: frame.spsr,
        }
    }

    fn restore(&self, frame: &mut ExceptionFrame) {
        frame.registers = self.registers;
        frame.simd = self.simd;
        frame.floating_point_control = self.floating_point_control;
        frame.floating_point_status = self.floating_point_status;
        frame.thread_pointer = self.thread_pointer;
        frame.read_only_thread_pointer = self.read_only_thread_pointer;
        frame.stack_pointer = self.stack_pointer;
        frame.elr = self.program_counter;
        frame.spsr = self.processor_state;
    }
}

/// Owns context transfers and their boot-time diagnostic state.
pub(crate) struct ContextSwitcher {
    self_test_handled: AtomicBool,
}

impl ContextSwitcher {
    pub(crate) const fn new() -> Self {
        Self {
            self_test_handled: AtomicBool::new(false),
        }
    }

    /// Saves the interrupted context and selects the context restored by `eret`.
    pub(crate) fn switch(
        &self,
        frame: &mut ExceptionFrame,
        outgoing: &mut RegisterContext,
        incoming: &RegisterContext,
    ) {
        self.save(frame, outgoing);
        self.restore(frame, incoming);
    }

    pub(crate) fn save(&self, frame: &ExceptionFrame, context: &mut RegisterContext) {
        *context = RegisterContext::capture(frame);
    }

    pub(crate) fn restore(&self, frame: &mut ExceptionFrame, context: &RegisterContext) {
        context.restore(frame);
    }

    pub(crate) fn handle_self_test(&self, frame: &mut ExceptionFrame) {
        let expected_stack_pointer = frame.stack_pointer;
        let expected_program_counter = frame.elr;
        let expected_processor_state = frame.spsr;
        let expected_floating_point_control = frame.floating_point_control;
        let expected_floating_point_status = frame.floating_point_status;
        let expected_thread_pointer = frame.thread_pointer;
        let expected_read_only_thread_pointer = frame.read_only_thread_pointer;
        let mut incoming = RegisterContext::capture(frame);
        incoming.set_register(0, TEST_OUTPUT);

        let mut outgoing = RegisterContext::empty();
        self.switch(frame, &mut outgoing, &incoming);
        let context_matches = outgoing.register(0) == TEST_INPUT
            && outgoing.stack_pointer() == expected_stack_pointer
            && outgoing.program_counter() == expected_program_counter
            && outgoing.processor_state() == expected_processor_state
            && outgoing.floating_point_control == expected_floating_point_control
            && outgoing.floating_point_status == expected_floating_point_status
            && outgoing.thread_pointer == expected_thread_pointer
            && outgoing.read_only_thread_pointer == expected_read_only_thread_pointer;
        self.self_test_handled
            .store(context_matches, Ordering::Relaxed);
    }

    pub(crate) fn self_test(&self) -> bool {
        self.self_test_handled.store(false, Ordering::Relaxed);
        let result: u64;

        // SAFETY: the SVC handler saves this context, substitutes x0 in the
        // restored context, and resumes after the `svc` instruction.
        unsafe {
            asm!(
                "svc #{svc}",
                svc = const KernelCall::ContextSelfTest as u16,
                inout("x0") TEST_INPUT => result,
            )
        };

        self.self_test_handled.load(Ordering::Relaxed) && result == TEST_OUTPUT
    }
}

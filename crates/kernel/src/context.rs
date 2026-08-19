use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::exceptions::{CONTEXT_SELF_TEST_SVC, ExceptionFrame};

const REGISTER_COUNT: usize = 31;
const TEST_INPUT: u64 = 0x1234_5678_9abc_def0;
const TEST_OUTPUT: u64 = 0xfedc_ba98_7654_3210;

static SELF_TEST_HANDLED: AtomicBool = AtomicBool::new(false);

/// Register state required to suspend and resume one AArch64 execution context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegisterContext {
    registers: [u64; REGISTER_COUNT],
    stack_pointer: u64,
    program_counter: u64,
    processor_state: u64,
}

impl RegisterContext {
    pub const fn new(entry: usize, stack_pointer: usize) -> Self {
        Self {
            registers: [0; REGISTER_COUNT],
            stack_pointer: stack_pointer as u64,
            program_counter: entry as u64,
            processor_state: 0,
        }
    }

    pub const fn register(&self, index: usize) -> u64 {
        self.registers[index]
    }

    pub fn set_register(&mut self, index: usize, value: u64) {
        self.registers[index] = value;
    }

    pub const fn stack_pointer(&self) -> u64 {
        self.stack_pointer
    }

    pub const fn program_counter(&self) -> u64 {
        self.program_counter
    }

    pub const fn processor_state(&self) -> u64 {
        self.processor_state
    }

    fn capture(frame: &ExceptionFrame) -> Self {
        Self {
            registers: frame.registers,
            stack_pointer: frame.stack_pointer,
            program_counter: frame.elr,
            processor_state: frame.spsr,
        }
    }

    fn restore(&self, frame: &mut ExceptionFrame) {
        frame.registers = self.registers;
        frame.stack_pointer = self.stack_pointer;
        frame.elr = self.program_counter;
        frame.spsr = self.processor_state;
    }
}

/// Saves the interrupted context and selects the context restored by `eret`.
pub(crate) fn switch(
    frame: &mut ExceptionFrame,
    outgoing: &mut RegisterContext,
    incoming: &RegisterContext,
) {
    *outgoing = RegisterContext::capture(frame);
    incoming.restore(frame);
}

pub(crate) fn handle_self_test(frame: &mut ExceptionFrame) {
    let expected_stack_pointer = frame.stack_pointer;
    let expected_program_counter = frame.elr;
    let expected_processor_state = frame.spsr;
    let mut incoming = RegisterContext::capture(frame);
    incoming.set_register(0, TEST_OUTPUT);

    let mut outgoing = RegisterContext::new(0, 0);
    switch(frame, &mut outgoing, &incoming);
    let context_matches = outgoing.register(0) == TEST_INPUT
        && outgoing.stack_pointer() == expected_stack_pointer
        && outgoing.program_counter() == expected_program_counter
        && outgoing.processor_state() == expected_processor_state;
    SELF_TEST_HANDLED.store(context_matches, Ordering::Relaxed);
}

pub fn self_test() -> bool {
    SELF_TEST_HANDLED.store(false, Ordering::Relaxed);
    let result: u64;

    // SAFETY: the dedicated SVC handler saves this context, substitutes x0 in
    // the restored context, and resumes at the instruction following `svc`.
    unsafe {
        asm!(
            "svc #{svc}",
            svc = const CONTEXT_SELF_TEST_SVC,
            inout("x0") TEST_INPUT => result,
            options(nostack)
        )
    };

    SELF_TEST_HANDLED.load(Ordering::Relaxed) && result == TEST_OUTPUT
}

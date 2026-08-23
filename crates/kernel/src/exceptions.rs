//! AArch64 exception-vector setup, frame layout, and central dispatch.

use core::arch::asm;
use core::fmt::Write;
use core::mem::{offset_of, size_of};
use core::ptr::addr_of;
use core::sync::atomic::{AtomicBool, Ordering};

use trains_platform::Platform;

use crate::Kernel;
use crate::svc::{KernelCall, UserCall};

const SVC64_EXCEPTION_CLASS: u64 = 0x15;

unsafe extern "C" {
    static vector_table: u8;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u64)]
enum Vector {
    CurrentSp0Sync = 0,
    CurrentSp0Irq = 1,
    CurrentSp0Fiq = 2,
    CurrentSp0SError = 3,
    CurrentSpxSync = 4,
    CurrentSpxIrq = 5,
    CurrentSpxFiq = 6,
    CurrentSpxSError = 7,
    LowerA64Sync = 8,
    LowerA64Irq = 9,
    LowerA64Fiq = 10,
    LowerA64SError = 11,
    LowerA32Sync = 12,
    LowerA32Irq = 13,
    LowerA32Fiq = 14,
    LowerA32SError = 15,
}

impl TryFrom<u64> for Vector {
    type Error = ();

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::CurrentSp0Sync),
            1 => Ok(Self::CurrentSp0Irq),
            2 => Ok(Self::CurrentSp0Fiq),
            3 => Ok(Self::CurrentSp0SError),
            4 => Ok(Self::CurrentSpxSync),
            5 => Ok(Self::CurrentSpxIrq),
            6 => Ok(Self::CurrentSpxFiq),
            7 => Ok(Self::CurrentSpxSError),
            8 => Ok(Self::LowerA64Sync),
            9 => Ok(Self::LowerA64Irq),
            10 => Ok(Self::LowerA64Fiq),
            11 => Ok(Self::LowerA64SError),
            12 => Ok(Self::LowerA32Sync),
            13 => Ok(Self::LowerA32Irq),
            14 => Ok(Self::LowerA32Fiq),
            15 => Ok(Self::LowerA32SError),
            _ => Err(()),
        }
    }
}

impl Vector {
    fn is_irq(self) -> bool {
        (self as u64) & 0b11 == 1
    }
}

#[repr(C)]
pub struct ExceptionFrame {
    pub(crate) registers: [u64; 31],
    pub(crate) stack_pointer: u64,
    pub(crate) elr: u64,
    pub(crate) spsr: u64,
    pub(crate) esr: u64,
    pub(crate) far: u64,
    pub(crate) vector: u64,
    reserved: u64,
    pub(crate) floating_point_control: u64,
    pub(crate) floating_point_status: u64,
    pub(crate) thread_pointer: u64,
    pub(crate) read_only_thread_pointer: u64,
    pub(crate) simd: [u128; 32],
}

const _: () = {
    assert!(size_of::<ExceptionFrame>() == 848);
    assert!(offset_of!(ExceptionFrame, registers) == 0);
    assert!(offset_of!(ExceptionFrame, stack_pointer) == 248);
    assert!(offset_of!(ExceptionFrame, elr) == 256);
    assert!(offset_of!(ExceptionFrame, spsr) == 264);
    assert!(offset_of!(ExceptionFrame, esr) == 272);
    assert!(offset_of!(ExceptionFrame, far) == 280);
    assert!(offset_of!(ExceptionFrame, vector) == 288);
    assert!(offset_of!(ExceptionFrame, floating_point_control) == 304);
    assert!(offset_of!(ExceptionFrame, floating_point_status) == 312);
    assert!(offset_of!(ExceptionFrame, thread_pointer) == 320);
    assert!(offset_of!(ExceptionFrame, read_only_thread_pointer) == 328);
    assert!(offset_of!(ExceptionFrame, simd) == 336);
};

/// Owns exception-vector setup, masking, diagnostics, and dispatch.
pub(crate) struct ExceptionHandler {
    self_test_handled: AtomicBool,
}

impl ExceptionHandler {
    pub(crate) const fn new() -> Self {
        Self {
            self_test_handled: AtomicBool::new(false),
        }
    }

    pub(crate) fn init(&self) {
        let vectors = addr_of!(vector_table) as u64;
        debug_assert_eq!(vectors & 0x7ff, 0);

        // SAFETY: `vector_table` is a 2 KiB-aligned, statically allocated
        // AArch64 vector table that remains valid for the kernel's lifetime.
        unsafe {
            asm!("msr vbar_el1, {vectors}", vectors = in(reg) vectors, options(nostack));
            asm!("isb", options(nostack, preserves_flags));
        }
    }

    pub(crate) fn self_test(&self) -> bool {
        self.self_test_handled.store(false, Ordering::Relaxed);
        // SAFETY: this immediate is reserved for the exception-entry self-test.
        // The handler recognizes it and returns after the `svc` instruction.
        unsafe { asm!("svc #{svc}", svc = const KernelCall::ExceptionSelfTest as u16) };
        self.self_test_handled.load(Ordering::Relaxed)
    }

    pub(crate) fn enable(&self) {
        // SAFETY: vector entry and IRQ dispatch are initialized before this is
        // called. Only IRQ is unmasked; FIQ, SError, and debug stay masked.
        unsafe { asm!("msr daifclr, #2", options(nomem, nostack, preserves_flags)) };
    }

    pub(crate) fn disable(&self) {
        // SAFETY: masking IRQ delivery is always safe and protects kernel state
        // that is not yet designed for concurrent interrupt mutation.
        unsafe { asm!("msr daifset, #2", options(nomem, nostack, preserves_flags)) };
    }

    pub(crate) fn handle<P: Platform>(&self, kernel: &Kernel<P>, frame: &mut ExceptionFrame) {
        let vector = Vector::try_from(frame.vector).ok();

        if vector.is_some_and(Vector::is_irq) {
            kernel.interrupt_handler.handle(kernel.platform);
            return;
        }

        let exception_class = frame.esr >> 26;
        let svc_number = frame.esr as u16;
        if exception_class == SVC64_EXCEPTION_CLASS
            && self.handle_supervisor_call(kernel, frame, vector, svc_number)
        {
            return;
        }

        self.handle_unhandled(kernel, frame, vector);
    }

    fn handle_supervisor_call<P: Platform>(
        &self,
        kernel: &Kernel<P>,
        frame: &mut ExceptionFrame,
        vector: Option<Vector>,
        number: u16,
    ) -> bool {
        match vector {
            Some(Vector::CurrentSpxSync) => self.decode_kernel_call(kernel, frame, number),
            Some(Vector::LowerA64Sync) => self.decode_user_call(kernel, frame, number),
            _ => false,
        }
    }

    fn decode_kernel_call<P: Platform>(
        &self,
        kernel: &Kernel<P>,
        frame: &mut ExceptionFrame,
        number: u16,
    ) -> bool {
        match KernelCall::try_from(number) {
            Ok(call) => {
                self.handle_kernel_call(kernel, frame, call);
                true
            }
            Err(_) => false,
        }
    }

    fn handle_kernel_call<P: Platform>(
        &self,
        kernel: &Kernel<P>,
        frame: &mut ExceptionFrame,
        call: KernelCall,
    ) {
        match call {
            KernelCall::ExceptionSelfTest => self.self_test_handled.store(true, Ordering::Relaxed),
            KernelCall::ContextSelfTest => kernel.context_switcher.handle_self_test(frame),
            KernelCall::StartScheduler => {
                kernel.scheduler.start_from(&kernel.context_switcher, frame)
            }
        }
    }

    fn decode_user_call<P: Platform>(
        &self,
        kernel: &Kernel<P>,
        frame: &mut ExceptionFrame,
        number: u16,
    ) -> bool {
        match UserCall::try_from(number) {
            Ok(call) => {
                self.handle_user_call(kernel, frame, call);
                true
            }
            Err(_) => false,
        }
    }

    fn handle_user_call<P: Platform>(
        &self,
        kernel: &Kernel<P>,
        frame: &mut ExceptionFrame,
        call: UserCall,
    ) {
        match call {
            UserCall::Yield => kernel
                .scheduler
                .yield_current(&kernel.context_switcher, frame),
            UserCall::Exit => kernel
                .scheduler
                .exit_current(&kernel.context_switcher, frame),
        }
    }

    fn handle_unhandled<P: Platform>(
        &self,
        kernel: &Kernel<P>,
        frame: &ExceptionFrame,
        vector: Option<Vector>,
    ) -> ! {
        let mut console = kernel.console();
        writeln!(console, "trains-rs: unhandled exception").ok();
        writeln!(console, "  vector: {:?}", vector).ok();
        writeln!(console, "  elr:    {:#018x}", frame.elr).ok();
        writeln!(console, "  spsr:   {:#018x}", frame.spsr).ok();
        writeln!(console, "  esr:    {:#018x}", frame.esr).ok();
        writeln!(console, "  far:    {:#018x}", frame.far).ok();

        loop {
            // SAFETY: an unhandled exception is fatal until a policy exists.
            unsafe { asm!("wfe", options(nomem, nostack, preserves_flags)) };
        }
    }
}

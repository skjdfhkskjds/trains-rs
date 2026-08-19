use core::arch::asm;
use core::fmt::Write;
use core::ptr::addr_of;
use core::sync::atomic::{AtomicBool, Ordering};

use trains_platform::{Platform as _, RASPI4};

const SVC64_EXCEPTION_CLASS: u64 = 0x15;
const SELF_TEST_SVC: u16 = 0x54;

static SELF_TEST_HANDLED: AtomicBool = AtomicBool::new(false);

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

impl Vector {
    fn from_raw(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::CurrentSp0Sync),
            1 => Some(Self::CurrentSp0Irq),
            2 => Some(Self::CurrentSp0Fiq),
            3 => Some(Self::CurrentSp0SError),
            4 => Some(Self::CurrentSpxSync),
            5 => Some(Self::CurrentSpxIrq),
            6 => Some(Self::CurrentSpxFiq),
            7 => Some(Self::CurrentSpxSError),
            8 => Some(Self::LowerA64Sync),
            9 => Some(Self::LowerA64Irq),
            10 => Some(Self::LowerA64Fiq),
            11 => Some(Self::LowerA64SError),
            12 => Some(Self::LowerA32Sync),
            13 => Some(Self::LowerA32Irq),
            14 => Some(Self::LowerA32Fiq),
            15 => Some(Self::LowerA32SError),
            _ => None,
        }
    }

    fn is_synchronous(self) -> bool {
        (self as u64) & 0b11 == 0
    }
}

#[repr(C)]
struct ExceptionFrame {
    registers: [u64; 31],
    elr: u64,
    spsr: u64,
    esr: u64,
    far: u64,
    vector: u64,
}

pub fn init() {
    let vectors = addr_of!(vector_table) as u64;
    debug_assert_eq!(vectors & 0x7ff, 0);

    // SAFETY: `vector_table` is a 2 KiB-aligned, statically allocated AArch64
    // vector table that remains valid for the lifetime of the kernel.
    unsafe {
        asm!("msr vbar_el1, {vectors}", vectors = in(reg) vectors, options(nostack));
        asm!("isb", options(nostack, preserves_flags));
    }
}

pub fn self_test() -> bool {
    SELF_TEST_HANDLED.store(false, Ordering::Relaxed);
    // SAFETY: this immediate is reserved for the exception-entry self-test.
    // The handler recognizes it and returns to the instruction after `svc`.
    unsafe { asm!("svc #0x54", options(nomem, nostack)) };
    SELF_TEST_HANDLED.load(Ordering::Relaxed)
}

#[unsafe(no_mangle)]
extern "C" fn exception_handler(frame: &mut ExceptionFrame) {
    let vector = Vector::from_raw(frame.vector);
    let exception_class = frame.esr >> 26;
    let syndrome = frame.esr as u32;

    if vector.is_some_and(Vector::is_synchronous)
        && exception_class == SVC64_EXCEPTION_CLASS
        && syndrome as u16 == SELF_TEST_SVC
    {
        SELF_TEST_HANDLED.store(true, Ordering::Relaxed);
        return;
    }

    let mut console = RASPI4.console();
    writeln!(console, "trains-rs: unhandled exception").ok();
    writeln!(console, "  vector: {:?}", vector).ok();
    writeln!(console, "  elr:    {:#018x}", frame.elr).ok();
    writeln!(console, "  spsr:   {:#018x}", frame.spsr).ok();
    writeln!(console, "  esr:    {:#018x}", frame.esr).ok();
    writeln!(console, "  far:    {:#018x}", frame.far).ok();

    loop {
        // SAFETY: an unhandled exception is fatal until a kernel policy exists.
        unsafe { asm!("wfe", options(nomem, nostack, preserves_flags)) };
    }
}

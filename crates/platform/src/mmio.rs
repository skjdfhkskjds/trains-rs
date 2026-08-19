//! Minimal volatile access primitives for memory-mapped devices.

use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};

#[inline(always)]
pub unsafe fn read32(address: usize) -> u32 {
    // SAFETY: the caller guarantees that `address` names a readable 32-bit
    // memory-mapped register.
    unsafe { read_volatile(address as *const u32) }
}

#[inline(always)]
pub unsafe fn write32(address: usize, value: u32) {
    // SAFETY: the caller guarantees that `address` names a writable 32-bit
    // memory-mapped register.
    unsafe { write_volatile(address as *mut u32, value) };
}

/// Completes explicit device accesses before execution continues.
#[inline(always)]
pub fn device_barrier() {
    // SAFETY: `dsb sy` has no operands and only orders memory accesses.
    unsafe { asm!("dsb sy", options(nostack, preserves_flags)) };
}

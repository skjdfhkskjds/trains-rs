use crate::mmio::{device_barrier, read32, write32};
use crate::{Interrupt, InterruptController};

const GICD_BASE: usize = 0xff84_1000;
const GICC_BASE: usize = 0xff84_2000;

const GICD_CTLR: usize = 0x000;
const GICD_IGROUPR: usize = 0x080;
const GICD_ISENABLER: usize = 0x100;
const GICD_ICENABLER: usize = 0x180;
const GICD_ISPENDR: usize = 0x200;
const GICD_IPRIORITYR: usize = 0x400;
const GICD_ITARGETSR: usize = 0x800;

const GICC_CTLR: usize = 0x000;
const GICC_PMR: usize = 0x004;

const CPU0_TARGET: u32 = 1;
const DEFAULT_PRIORITY: u32 = 0xa0;

#[derive(Clone, Copy)]
pub struct Gic400 {
    distributor: usize,
    cpu_interface: usize,
}

pub(super) const INTERRUPTS: Gic400 = Gic400 {
    distributor: GICD_BASE,
    cpu_interface: GICC_BASE,
};

impl Gic400 {
    pub(super) fn init(&self) {
        self.write_distributor(GICD_CTLR, 0);

        // Permit every priority at the CPU interface, then enable Group 0.
        // CPU IRQs remain masked in DAIF until exception vectors exist.
        self.write_cpu(GICC_PMR, 0xff);
        self.write_cpu(GICC_CTLR, 1);
        self.write_distributor(GICD_CTLR, 1);
        device_barrier();
    }

    fn configure(&self, interrupt: Interrupt) {
        let id = interrupt as usize;
        let byte_shift = (id % 4) * 8;

        // Raspberry Pi firmware and QEMU enter the kernel in the non-secure
        // world. Assign the SPI to Group 1 so it is visible there.
        let group = self.distributor + GICD_IGROUPR + (id / 32) * 4;
        // SAFETY: the interrupt ID selects its packed GIC group register.
        let value = unsafe { read32(group) } | (1 << (id % 32));
        // SAFETY: same group register as above.
        unsafe { write32(group, value) };

        let priority = self.distributor + GICD_IPRIORITYR + (id / 4) * 4;
        // SAFETY: the interrupt ID selects its packed GIC priority register.
        let mut value = unsafe { read32(priority) };
        value = (value & !(0xff << byte_shift)) | (DEFAULT_PRIORITY << byte_shift);
        // SAFETY: same priority register as above.
        unsafe { write32(priority, value) };

        if id >= 32 {
            let targets = self.distributor + GICD_ITARGETSR + (id / 4) * 4;
            // SAFETY: the SPI interrupt ID selects its packed target register.
            let mut value = unsafe { read32(targets) };
            value = (value & !(0xff << byte_shift)) | (CPU0_TARGET << byte_shift);
            // SAFETY: same target register as above.
            unsafe { write32(targets, value) };
        }
    }

    fn bit_register(&self, offset: usize, interrupt: Interrupt) -> (usize, u32) {
        let id = interrupt as usize;
        (self.distributor + offset + (id / 32) * 4, 1 << (id % 32))
    }

    #[inline]
    fn write_distributor(&self, offset: usize, value: u32) {
        // SAFETY: every call uses a GIC-400 distributor register offset.
        unsafe { write32(self.distributor + offset, value) };
    }

    #[inline]
    fn write_cpu(&self, offset: usize, value: u32) {
        // SAFETY: every call uses a GIC-400 CPU-interface register offset.
        unsafe { write32(self.cpu_interface + offset, value) };
    }
}

impl InterruptController for Gic400 {
    fn enable(&self, interrupt: Interrupt) {
        self.configure(interrupt);
        let (register, mask) = self.bit_register(GICD_ISENABLER, interrupt);
        // SAFETY: `bit_register` returns the interrupt's set-enable register.
        unsafe { write32(register, mask) };
        device_barrier();
    }

    fn disable(&self, interrupt: Interrupt) {
        let (register, mask) = self.bit_register(GICD_ICENABLER, interrupt);
        // SAFETY: `bit_register` returns the interrupt's clear-enable register.
        unsafe { write32(register, mask) };
        device_barrier();
    }

    fn is_pending(&self, interrupt: Interrupt) -> bool {
        let (register, mask) = self.bit_register(GICD_ISPENDR, interrupt);
        // SAFETY: `bit_register` returns the interrupt's pending register.
        unsafe { read32(register) & mask != 0 }
    }
}

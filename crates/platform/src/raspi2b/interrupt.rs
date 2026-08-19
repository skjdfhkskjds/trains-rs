use crate::mmio::{device_barrier, read32, write32};
use crate::{Interrupt, InterruptController};

use super::{LOCAL_PERIPHERAL_BASE, PERIPHERAL_BASE};

const ARM_CONTROL_BASE: usize = PERIPHERAL_BASE + 0xb000;
const IRQ_PENDING_1: usize = 0x204;
const IRQ_PENDING_2: usize = 0x208;
const ENABLE_IRQS_1: usize = 0x210;
const ENABLE_IRQS_2: usize = 0x214;
const DISABLE_IRQS_1: usize = 0x21c;
const DISABLE_IRQS_2: usize = 0x220;

const GPU_INTERRUPT_ROUTING: usize = LOCAL_PERIPHERAL_BASE + 0x0c;

#[derive(Clone, Copy)]
pub struct Bcm2835InterruptController {
    base: usize,
}

pub(super) const INTERRUPTS: Bcm2835InterruptController = Bcm2835InterruptController {
    base: ARM_CONTROL_BASE,
};

impl Bcm2835InterruptController {
    pub(super) fn route_peripheral_irqs_to_core(&self, core: u8) {
        assert!(core < 4, "invalid BCM2836 core number");

        // Bits 1:0 select the core that receives the chained peripheral IRQ.
        // Preserve the independent FIQ route in bits 3:2.
        // SAFETY: this is the BCM2836 GPU interrupt-routing register.
        let current = unsafe { read32(GPU_INTERRUPT_ROUTING) };

        // SAFETY: same routing register as above.
        unsafe { write32(GPU_INTERRUPT_ROUTING, (current & !0b11) | u32::from(core)) };
        device_barrier();
    }

    fn bank_register(
        &self,
        interrupt: Interrupt,
        bank_one_offset: usize,
        bank_two_offset: usize,
    ) -> (usize, u32) {
        let id = interrupt as u8;
        if id < 32 {
            (self.base + bank_one_offset, 1 << id)
        } else {
            (self.base + bank_two_offset, 1 << (id - 32))
        }
    }
}

impl InterruptController for Bcm2835InterruptController {
    fn enable(&self, interrupt: Interrupt) {
        let (register, mask) = self.bank_register(interrupt, ENABLE_IRQS_1, ENABLE_IRQS_2);

        // SAFETY: `bank_register` returns a valid interrupt-enable register.
        unsafe { write32(register, mask) };
        device_barrier();
    }

    fn disable(&self, interrupt: Interrupt) {
        let (register, mask) = self.bank_register(interrupt, DISABLE_IRQS_1, DISABLE_IRQS_2);

        // SAFETY: `bank_register` returns a valid interrupt-disable register.
        unsafe { write32(register, mask) };
        device_barrier();
    }

    fn is_pending(&self, interrupt: Interrupt) -> bool {
        let (register, mask) = self.bank_register(interrupt, IRQ_PENDING_1, IRQ_PENDING_2);
        
        // SAFETY: `bank_register` returns a valid pending register.
        unsafe { read32(register) & mask != 0 }
    }
}

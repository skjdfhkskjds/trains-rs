pub trait DelayNs {
    fn delay_ns(&mut self, nanoseconds: u32);

    fn delay_us(&mut self, microseconds: u32) {
        for _ in 0..microseconds / 1_000_000 {
            self.delay_ns(1_000_000_000);
        }
        self.delay_ns((microseconds % 1_000_000) * 1_000);
    }

    fn delay_ms(&mut self, milliseconds: u32) {
        for _ in 0..milliseconds / 1_000 {
            self.delay_ns(1_000_000_000);
        }
        self.delay_ns((milliseconds % 1_000) * 1_000_000);
    }
}

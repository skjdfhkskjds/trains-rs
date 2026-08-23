//! Fixed-capacity FIFO storage used by the cooperative scheduling policy.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct QueueFull;

pub(super) struct ReadyQueue<T: Copy, const CAPACITY: usize> {
    entries: [Option<T>; CAPACITY],
    head: usize,
    len: usize,
}

impl<T: Copy, const CAPACITY: usize> ReadyQueue<T, CAPACITY> {
    pub(super) const fn new() -> Self {
        Self {
            entries: [None; CAPACITY],
            head: 0,
            len: 0,
        }
    }

    pub(super) const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(super) fn push_back(&mut self, entry: T) -> Result<(), QueueFull> {
        if self.len == CAPACITY {
            return Err(QueueFull);
        }

        let tail = (self.head + self.len) % CAPACITY;
        self.entries[tail] = Some(entry);
        self.len += 1;
        Ok(())
    }

    pub(super) fn pop_front(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }

        let entry = self.entries[self.head].take();
        self.head = (self.head + 1) % CAPACITY;
        self.len -= 1;
        entry
    }
}

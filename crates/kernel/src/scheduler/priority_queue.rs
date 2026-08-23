//! Fixed-capacity stable priority queue used by the scheduler.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct QueueFull;

#[derive(Clone, Copy)]
struct Entry<T, P> {
    value: T,
    priority: P,
}

pub(super) struct PriorityQueue<T: Copy, P: Copy + Ord, const CAPACITY: usize> {
    entries: [Option<Entry<T, P>>; CAPACITY],
    len: usize,
}

impl<T: Copy, P: Copy + Ord, const CAPACITY: usize> PriorityQueue<T, P, CAPACITY> {
    pub(super) const fn new() -> Self {
        Self {
            entries: [None; CAPACITY],
            len: 0,
        }
    }

    pub(super) const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Inserts after existing entries of the same priority, preserving FIFO
    /// scheduling among equally prioritized tasks.
    pub(super) fn push(&mut self, value: T, priority: P) -> Result<(), QueueFull> {
        if self.len == CAPACITY {
            return Err(QueueFull);
        }

        let insertion_index = self.entries[..self.len]
            .iter()
            .position(|entry| {
                entry
                    .as_ref()
                    .is_some_and(|entry| entry.priority > priority)
            })
            .unwrap_or(self.len);

        for index in (insertion_index..self.len).rev() {
            self.entries[index + 1] = self.entries[index].take();
        }
        self.entries[insertion_index] = Some(Entry { value, priority });
        self.len += 1;
        Ok(())
    }

    pub(super) fn pop(&mut self) -> Option<(T, P)> {
        let entry = self.entries.first_mut()?.take()?;

        for index in 1..self.len {
            self.entries[index - 1] = self.entries[index].take();
        }
        self.len -= 1;
        Some((entry.value, entry.priority))
    }
}

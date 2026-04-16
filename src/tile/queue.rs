//! Priority queue with pluggable ordering and size limit.
//! Generic — does not know about tiles, coordinates, or zoom levels.

use std::cmp::Ordering;

const MAX_QUEUE_SIZE: usize = 16;

/// Assigns priority to items. Lower value = higher priority.
pub trait PriorityFn<T>: Send {
    fn priority(&self, item: &T) -> f64;
}

/// Blanket impl: any closure `Fn(&T) -> f64 + Send` works as a PriorityFn.
impl<T, F: Fn(&T) -> f64 + Send> PriorityFn<T> for F {
    fn priority(&self, item: &T) -> f64 {
        self(item)
    }
}

struct Entry<T> {
    item: T,
    priority: f64,
}

pub struct PriorityQueue<T: PartialEq> {
    entries: Vec<Entry<T>>,
    max_size: usize,
}

impl<T: PartialEq> PriorityQueue<T> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            max_size: MAX_QUEUE_SIZE,
        }
    }

    /// Push an item with a given priority. Skips duplicates.
    /// Drops the lowest priority item if full.
    pub fn push(&mut self, item: T, priority: f64) {
        if self.entries.iter().any(|e| e.item == item) {
            return;
        }

        if self.entries.len() >= self.max_size {
            // Drop lowest priority (highest value = last after sort)
            self.entries.pop();
        }

        let pos = self
            .entries
            .binary_search_by(|e| e.priority.partial_cmp(&priority).unwrap_or(Ordering::Equal))
            .unwrap_or_else(|i| i);
        self.entries.insert(pos, Entry { item, priority });
    }

    /// Pop the highest priority item (lowest priority value).
    pub fn pop(&mut self) -> Option<T> {
        if self.entries.is_empty() {
            None
        } else {
            Some(self.entries.remove(0).item)
        }
    }

    /// Remove items that don't pass the predicate. Re-sort with new priorities.
    pub fn retain_and_reprioritize<F, P>(&mut self, retain: F, priority_fn: &P)
    where
        F: Fn(&T) -> bool,
        P: PriorityFn<T>,
    {
        self.entries.retain(|e| retain(&e.item));
        for entry in &mut self.entries {
            entry.priority = priority_fn.priority(&entry.item);
        }
        self.entries.sort_by(|a, b| {
            a.priority
                .partial_cmp(&b.priority)
                .unwrap_or(Ordering::Equal)
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<T: PartialEq> Default for PriorityQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lowest_priority_value_first() {
        let mut q = PriorityQueue::new();
        q.push("far", 100.0);
        q.push("close", 1.0);
        q.push("medium", 50.0);

        assert_eq!(q.pop(), Some("close"));
        assert_eq!(q.pop(), Some("medium"));
        assert_eq!(q.pop(), Some("far"));
    }

    #[test]
    fn test_overflow_drops_lowest_priority() {
        let mut q = PriorityQueue::<i32>::new();
        for i in 0..MAX_QUEUE_SIZE as i32 {
            q.push(i, i as f64); // higher i = lower priority
        }
        assert_eq!(q.len(), MAX_QUEUE_SIZE);

        // Push high-priority item — should displace lowest
        q.push(999, -1.0);
        assert_eq!(q.len(), MAX_QUEUE_SIZE);
        assert_eq!(q.pop(), Some(999)); // highest priority
    }

    #[test]
    fn test_skip_duplicates() {
        let mut q = PriorityQueue::new();
        q.push(42, 1.0);
        q.push(42, 2.0);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn test_retain_and_reprioritize() {
        let mut q = PriorityQueue::new();
        q.push(1, 10.0);
        q.push(2, 20.0);
        q.push(3, 30.0);
        q.push(4, 40.0);

        // Keep only even, reprioritize: smaller = higher priority
        q.retain_and_reprioritize(|x| x % 2 == 0, &|x: &i32| *x as f64);
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop(), Some(2));
        assert_eq!(q.pop(), Some(4));
    }
}

//! Priority queue with pluggable ordering and size limit.
//! Generic — does not know about tiles, coordinates, or zoom levels.

use std::cmp::Ordering;

const MAX_QUEUE_SIZE: usize = 16;

/// Assigns a priority to items. Lower = higher priority.
/// `P` is any type with `PartialOrd` (e.g. `f64`, a tuple, or a struct
/// with derived `PartialOrd`). This lets callers encode richer ordering
/// than a single scalar — e.g. `(zoom_diff, distance)` for tile fetch.
pub trait PriorityFn<T, P>: Send {
    fn priority(&self, item: &T) -> P;
}

/// Blanket impl: any closure `Fn(&T) -> P + Send` works as a `PriorityFn`.
impl<T, P, F: Fn(&T) -> P + Send> PriorityFn<T, P> for F {
    fn priority(&self, item: &T) -> P {
        self(item)
    }
}

struct Entry<T, P> {
    item: T,
    priority: P,
}

pub struct PriorityQueue<T: PartialEq, P: PartialOrd> {
    entries: Vec<Entry<T, P>>,
    max_size: usize,
}

impl<T: PartialEq, P: PartialOrd> PriorityQueue<T, P> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            max_size: MAX_QUEUE_SIZE,
        }
    }

    /// Push an item with a given priority. Skips duplicates.
    /// Drops the lowest priority item if full.
    pub fn push(&mut self, item: T, priority: P) {
        if self.entries.iter().any(|e| e.item == item) {
            return;
        }

        if self.entries.len() >= self.max_size {
            // Drop lowest priority (ends up last after sort).
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

    /// Recompute every entry's priority and re-sort.
    pub fn reprioritize<F>(&mut self, priority_fn: &F)
    where
        F: PriorityFn<T, P> + ?Sized,
    {
        for entry in &mut self.entries {
            entry.priority = priority_fn.priority(&entry.item);
        }
        self.entries.sort_by(|a, b| {
            a.priority
                .partial_cmp(&b.priority)
                .unwrap_or(Ordering::Equal)
        });
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<T: PartialEq, P: PartialOrd> Default for PriorityQueue<T, P> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lowest_priority_value_first() {
        let mut q: PriorityQueue<&str, f64> = PriorityQueue::new();
        q.push("far", 100.0);
        q.push("close", 1.0);
        q.push("medium", 50.0);

        assert_eq!(q.pop(), Some("close"));
        assert_eq!(q.pop(), Some("medium"));
        assert_eq!(q.pop(), Some("far"));
    }

    #[test]
    fn test_overflow_drops_lowest_priority() {
        let mut q: PriorityQueue<i32, f64> = PriorityQueue::new();
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
        let mut q: PriorityQueue<i32, f64> = PriorityQueue::new();
        q.push(42, 1.0);
        q.push(42, 2.0);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn test_reprioritize() {
        let mut q: PriorityQueue<i32, f64> = PriorityQueue::new();
        q.push(1, 40.0);
        q.push(2, 30.0);
        q.push(3, 20.0);
        q.push(4, 10.0);

        // Reprioritize so smaller value = higher priority.
        q.reprioritize(&|x: &i32| *x as f64);
        assert_eq!(q.len(), 4);
        assert_eq!(q.pop(), Some(1));
        assert_eq!(q.pop(), Some(2));
        assert_eq!(q.pop(), Some(3));
        assert_eq!(q.pop(), Some(4));
    }

    /// Composite priority: (zoom_diff, distance). Smaller first field wins
    /// absolutely; second field tie-breaks. Derived `PartialOrd` compares
    /// fields in declaration order.
    #[derive(PartialEq, PartialOrd)]
    struct ZoomThenDistance {
        zoom_diff: u32,
        distance: f64,
    }

    #[test]
    fn test_composite_priority_zoom_dominates() {
        let mut q: PriorityQueue<&str, ZoomThenDistance> = PriorityQueue::new();

        // "far-current" has big distance but matches current zoom.
        // "near-stale" is very close but at a stale zoom.
        // Current zoom must win regardless of distance.
        q.push(
            "far-current",
            ZoomThenDistance {
                zoom_diff: 0,
                distance: 999.0,
            },
        );
        q.push(
            "near-stale",
            ZoomThenDistance {
                zoom_diff: 4,
                distance: 1.0,
            },
        );

        assert_eq!(q.pop(), Some("far-current"));
        assert_eq!(q.pop(), Some("near-stale"));
    }
}

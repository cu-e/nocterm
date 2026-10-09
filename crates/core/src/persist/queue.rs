//! An ordered, bounded queue of changes to one persisted file.
//!
//! The owner applies each change to the latest saved state only when it
//! reaches the front, so changes made in quick succession never conflict.
//! The queue holds no executor: its owner drains it with a task of its own and
//! marks every disk write with a [`Writing`] guard, which lets shutdown wait for
//! writes already in flight after the owner itself can no longer be read.

use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

/// Why a change was not queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// The application is quitting.
    Closing,
    /// `limit` changes are already waiting.
    Full,
}

pub struct WriteQueue<C> {
    pending: VecDeque<C>,
    limit: usize,
    draining: bool,
    closing: bool,
    writes: Arc<AtomicUsize>,
}

/// Marks one disk write as in flight until dropped.
#[must_use = "the write counts as in flight only while the guard lives"]
pub struct Writing(Arc<AtomicUsize>);

/// Counts the writes in flight, independently of the queue.
#[derive(Clone)]
pub struct InFlight(Arc<AtomicUsize>);

impl<C> WriteQueue<C> {
    /// A queue that holds at most `limit` waiting changes.
    pub fn new(limit: usize) -> Self {
        Self {
            pending: VecDeque::new(),
            limit,
            draining: false,
            closing: false,
            writes: Arc::default(),
        }
    }

    /// Queues `change` behind the others. Returns `true` when the queue was
    /// idle, so the caller must start a task that drains it with [`take`].
    ///
    /// [`take`]: Self::take
    pub fn push(&mut self, change: C) -> Result<bool, Rejected> {
        if self.closing {
            return Err(Rejected::Closing);
        }
        if self.pending.len() >= self.limit {
            return Err(Rejected::Full);
        }
        self.pending.push_back(change);
        Ok(!std::mem::replace(&mut self.draining, true))
    }

    /// Takes the oldest change. `None` ends the drain: the next [`push`]
    /// asks for a new one.
    ///
    /// [`push`]: Self::push
    pub fn take(&mut self) -> Option<C> {
        let change = self.pending.pop_front();
        self.draining = change.is_some();
        change
    }

    /// The changes waiting, not counting one being written.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn is_closing(&self) -> bool {
        self.closing
    }

    /// Stops accepting changes and returns how many waiting ones will never
    /// be written.
    pub fn close(&mut self) -> usize {
        self.closing = true;
        self.pending.len()
    }

    /// Marks a disk write as in flight until the guard drops. Writes made
    /// outside the queue, such as of related files, may use it too.
    pub fn begin_write(&self) -> Writing {
        self.writes.fetch_add(1, Ordering::AcqRel);
        Writing(self.writes.clone())
    }

    pub fn in_flight(&self) -> InFlight {
        InFlight(self.writes.clone())
    }
}

impl Drop for Writing {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl InFlight {
    pub fn count(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_change_of_a_drain_starts_one() {
        let mut queue = WriteQueue::new(4);
        assert_eq!(queue.push(1), Ok(true));
        assert_eq!(queue.push(2), Ok(false));
        assert_eq!(queue.take(), Some(1));
        assert_eq!(queue.push(3), Ok(false), "the drain is still running");
        assert_eq!(queue.take(), Some(2));
        assert_eq!(queue.take(), Some(3));
        assert_eq!(queue.take(), None);
        assert_eq!(queue.push(4), Ok(true), "the drain ended");
    }

    #[test]
    fn a_full_queue_rejects_until_a_change_leaves() {
        let mut queue = WriteQueue::new(2);
        queue.push(1).unwrap();
        queue.push(2).unwrap();
        assert_eq!(queue.push(3), Err(Rejected::Full));
        queue.take();
        assert_eq!(queue.push(3), Ok(false));
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn closing_rejects_new_changes_and_reports_the_lost_ones() {
        let mut queue = WriteQueue::new(4);
        queue.push(1).unwrap();
        queue.push(2).unwrap();
        assert_eq!(queue.close(), 2);
        assert!(queue.is_closing());
        assert_eq!(queue.push(3), Err(Rejected::Closing));
    }

    #[test]
    fn in_flight_counts_live_write_guards() {
        let queue = WriteQueue::<()>::new(1);
        let in_flight = queue.in_flight();
        let first = queue.begin_write();
        let second = queue.begin_write();
        assert_eq!(in_flight.count(), 2);
        drop(first);
        assert_eq!(in_flight.count(), 1);
        drop(queue);
        drop(second);
        assert_eq!(in_flight.count(), 0);
    }
}

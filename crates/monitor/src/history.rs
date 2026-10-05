//! Bounded time series, for graphs.

use std::collections::VecDeque;

/// The most recent values of one quantity, stamped with the host's clock.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Series {
    points: VecDeque<(f64, f32)>,
    capacity: usize,
}

impl Series {
    pub fn new(capacity: usize) -> Self {
        Self {
            points: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, time: f64, value: f32) {
        // A clock that went back is another boot: the old points mean nothing.
        if self.points.back().is_some_and(|(last, _)| *last > time) {
            self.points.clear();
        }
        if self.points.back().is_some_and(|(last, _)| *last == time) {
            self.points.pop_back();
        }
        self.points.push_back((time, value));
        self.trim();
    }

    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        self.trim();
    }

    /// `(time, value)` pairs, oldest first.
    pub fn points(&self) -> impl ExactSizeIterator<Item = (f64, f32)> + '_ {
        self.points.iter().copied()
    }

    pub fn last(&self) -> Option<f32> {
        self.points.back().map(|(_, value)| *value)
    }

    pub fn max(&self) -> f32 {
        self.points
            .iter()
            .map(|(_, value)| *value)
            .fold(0.0, f32::max)
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    fn trim(&mut self) {
        while self.points.len() > self.capacity.max(2) {
            self.points.pop_front();
        }
    }
}

/// The series the details graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    /// Total processor load, in percent.
    pub cpu: Series,
    /// Memory in use, in percent.
    pub memory: Series,
    /// Network bytes per second.
    pub received: Series,
    pub sent: Series,
    /// Disk bytes per second.
    pub read: Series,
    pub written: Series,
}

impl History {
    pub fn new(capacity: usize) -> Self {
        Self {
            cpu: Series::new(capacity),
            memory: Series::new(capacity),
            received: Series::new(capacity),
            sent: Series::new(capacity),
            read: Series::new(capacity),
            written: Series::new(capacity),
        }
    }

    pub(crate) fn set_capacity(&mut self, capacity: usize) {
        for series in [
            &mut self.cpu,
            &mut self.memory,
            &mut self.received,
            &mut self.sent,
            &mut self.read,
            &mut self.written,
        ] {
            series.set_capacity(capacity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_keep_the_newest_points_in_time_order() {
        let mut series = Series::new(3);
        for (time, value) in [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0), (4.0, 4.0)] {
            series.push(time, value);
        }
        assert_eq!(
            series.points().collect::<Vec<_>>(),
            [(2.0, 2.0), (3.0, 3.0), (4.0, 4.0)]
        );
        assert_eq!(series.max(), 4.0);
        series.push(4.0, 5.0);
        assert_eq!(series.last(), Some(5.0));
        assert_eq!(series.len(), 3);
        series.push(0.5, 1.0);
        assert_eq!(series.len(), 1);
        series.set_capacity(1);
        assert_eq!(series.len(), 1);
    }
}

//! Which metrics a watch collects.

use nocterm_settings::MonitorMetric;

/// A set of [`MonitorMetric`]s, cheap to copy and compare.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MetricSet(u16);

impl MetricSet {
    pub const EMPTY: Self = Self(0);

    pub fn all() -> Self {
        MonitorMetric::ALL.into_iter().collect()
    }

    pub fn contains(self, metric: MonitorMetric) -> bool {
        self.0 & bit(metric) != 0
    }

    pub fn insert(&mut self, metric: MonitorMetric) {
        self.0 |= bit(metric);
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether any of `metrics` is in the set.
    pub fn any(self, metrics: &[MonitorMetric]) -> bool {
        metrics.iter().any(|metric| self.contains(*metric))
    }

    pub fn iter(self) -> impl Iterator<Item = MonitorMetric> {
        MonitorMetric::ALL
            .into_iter()
            .filter(move |metric| self.contains(*metric))
    }
}

fn bit(metric: MonitorMetric) -> u16 {
    let index = MonitorMetric::ALL
        .iter()
        .position(|candidate| *candidate == metric)
        .unwrap_or_default();
    1 << index
}

impl FromIterator<MonitorMetric> for MetricSet {
    fn from_iter<I: IntoIterator<Item = MonitorMetric>>(metrics: I) -> Self {
        let mut set = Self::EMPTY;
        for metric in metrics {
            set.insert(metric);
        }
        set
    }
}

impl<'a> FromIterator<&'a MonitorMetric> for MetricSet {
    fn from_iter<I: IntoIterator<Item = &'a MonitorMetric>>(metrics: I) -> Self {
        metrics.into_iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_hold_exactly_what_was_inserted() {
        let set: MetricSet = [MonitorMetric::Cpu, MonitorMetric::DiskIo].iter().collect();
        assert!(set.contains(MonitorMetric::Cpu));
        assert!(set.contains(MonitorMetric::DiskIo));
        assert!(!set.contains(MonitorMetric::Memory));
        assert_eq!(set.iter().count(), 2);
        assert_eq!(MetricSet::all().iter().count(), MonitorMetric::ALL.len());
        assert!(MetricSet::EMPTY.is_empty());
        let both = set.union([MonitorMetric::Memory].iter().collect());
        assert!(both.any(&[MonitorMetric::Memory, MonitorMetric::Swap]));
    }
}

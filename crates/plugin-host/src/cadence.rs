//! How often the host samples observations and lets plugins draw.

use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

use gpui::WindowId;
use nocterm_plugin_api::{HostError, Observed};
use nocterm_plugin_model::PluginId;

use crate::caps::{MAX_RENDERS_PER_SECOND, OBSERVE_EVERY, OBSERVE_EVERY_FOCUSED};

type Sample = Result<Observed, HostError>;

/// What was last read for a plugin in a window, and when.
#[derive(Default)]
struct Samples {
    cache: BTreeMap<(WindowId, PluginId, &'static str), (Instant, Sample)>,
}

/// Sampling cadence and the render budget.
#[derive(Default)]
pub(crate) struct Cadence {
    samples: Samples,
    renders: VecDeque<Instant>,
}

impl Cadence {
    /// The last sample if it is younger than the cadence, else a fresh one from `read`.
    ///
    /// A plugin cannot ask faster than the cadence: the answer between two samples is the
    /// earlier one.
    pub(crate) fn sample(
        &mut self,
        key: (WindowId, &PluginId, &'static str),
        in_focus: bool,
        now: Instant,
        read: impl FnOnce() -> Sample,
    ) -> Sample {
        let every = if in_focus {
            OBSERVE_EVERY_FOCUSED
        } else {
            OBSERVE_EVERY
        };
        let key = (key.0, key.1.clone(), key.2);
        if let Some((at, sample)) = self.samples.cache.get(&key)
            && now.saturating_duration_since(*at) < every
        {
            return sample.clone();
        }
        let sample = read();
        self.samples.cache.insert(key, (now, sample.clone()));
        sample
    }

    /// Counts one render, or says when the budget has room again.
    pub(crate) fn admit_render(&mut self, now: Instant) -> Result<(), u32> {
        let second = Duration::from_secs(1);
        while self
            .renders
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= second)
        {
            self.renders.pop_front();
        }
        if self.renders.len() >= MAX_RENDERS_PER_SECOND
            && let Some(oldest) = self.renders.front()
        {
            let wait = second.saturating_sub(now.saturating_duration_since(*oldest));
            return Err(u32::try_from(wait.as_millis()).unwrap_or(u32::MAX).max(1));
        }
        self.renders.push_back(now);
        Ok(())
    }

    pub(crate) fn forget(&mut self, plugin: &PluginId) {
        self.samples
            .cache
            .retain(|(_, owner, _), _| owner != plugin);
    }
}

#[cfg(test)]
mod tests {
    use nocterm_plugin_api::capability::clock::ClockNow;

    use super::*;

    fn clock() -> Sample {
        Ok(Observed::Clock(ClockNow {
            unix_minutes: 1,
            utc_offset_seconds: 0,
        }))
    }

    #[test]
    fn a_sample_is_reused_until_the_cadence_has_passed() {
        let mut cadence = Cadence::default();
        let (window, plugin, now) = (
            WindowId::default(),
            PluginId::owned("user-a-b"),
            Instant::now(),
        );
        let mut reads = 0;
        let mut read = |cadence: &mut Cadence, at: Duration, in_focus: bool| {
            cadence.sample((window, &plugin, "clock.now"), in_focus, now + at, || {
                reads += 1;
                clock()
            })
        };
        read(&mut cadence, Duration::ZERO, false).unwrap();
        read(&mut cadence, Duration::from_millis(999), false).unwrap();
        read(&mut cadence, Duration::from_millis(1000), false).unwrap();
        read(&mut cadence, Duration::from_millis(1100), true).unwrap();
        read(&mut cadence, Duration::from_millis(1250), true).unwrap();
        assert_eq!(reads, 3);
    }

    #[test]
    fn a_refusal_is_sampled_at_the_same_cadence() {
        let mut cadence = Cadence::default();
        let (window, plugin, now) = (
            WindowId::default(),
            PluginId::owned("user-a-b"),
            Instant::now(),
        );
        let mut reads = 0;
        for _ in 0..5 {
            let sample = cadence.sample((window, &plugin, "monitor.usage"), false, now, || {
                reads += 1;
                Err(HostError::Unavailable)
            });
            assert_eq!(sample, Err(HostError::Unavailable));
        }
        assert_eq!(reads, 1);
    }

    #[test]
    fn renders_beyond_the_budget_wait_for_the_oldest_to_age_out() {
        let mut cadence = Cadence::default();
        let now = Instant::now();
        for _ in 0..MAX_RENDERS_PER_SECOND {
            assert_eq!(cadence.admit_render(now), Ok(()));
        }
        assert_eq!(
            cadence.admit_render(now + Duration::from_millis(400)),
            Err(600)
        );
        assert_eq!(cadence.admit_render(now + Duration::from_secs(1)), Ok(()));
    }
}

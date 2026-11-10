//! Per-plugin use limits taken from the capability table.

use std::{
    collections::{BTreeMap, VecDeque},
    time::Instant,
};

use nocterm_plugin_api::{CapabilityEntry, HostError};
use nocterm_plugin_model::PluginId;

/// The uses of each rate-limited capability, per plugin.
#[derive(Default)]
pub(crate) struct RateLimits {
    uses: BTreeMap<(PluginId, &'static str), VecDeque<Instant>>,
}

impl RateLimits {
    /// Counts one use of the capability, or says when the next one is allowed.
    pub(crate) fn admit(
        &mut self,
        plugin: &PluginId,
        entry: &'static CapabilityEntry,
        now: Instant,
    ) -> Result<(), HostError> {
        let Some((count, window)) = entry.rate else {
            return Ok(());
        };
        let uses = self.uses.entry((plugin.clone(), entry.id)).or_default();
        while uses
            .front()
            .is_some_and(|used| now.saturating_duration_since(*used) >= window)
        {
            uses.pop_front();
        }
        if uses.len() >= usize::from(count)
            && let Some(oldest) = uses.front()
        {
            let wait = window.saturating_sub(now.saturating_duration_since(*oldest));
            let retry_after_ms = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX).max(1);
            return Err(HostError::RateLimited { retry_after_ms });
        }
        uses.push_back(now);
        Ok(())
    }

    pub(crate) fn forget(&mut self, plugin: &PluginId) {
        self.uses.retain(|(owner, _), _| owner != plugin);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nocterm_plugin_api::capability::capability;

    use super::*;

    fn plugin(name: &str) -> PluginId {
        PluginId::owned(name.to_owned())
    }

    fn post() -> &'static CapabilityEntry {
        capability("notifications.post").unwrap()
    }

    #[test]
    fn a_capability_without_a_rate_is_never_limited() {
        let mut rates = RateLimits::default();
        let now = Instant::now();
        let storage = capability("storage.set").unwrap();
        for _ in 0..1000 {
            assert_eq!(rates.admit(&plugin("user-a-b"), storage, now), Ok(()));
        }
    }

    #[test]
    fn the_wait_shrinks_as_time_passes_and_ends_at_the_window() {
        let mut rates = RateLimits::default();
        let start = Instant::now();
        assert_eq!(rates.admit(&plugin("user-a-b"), post(), start), Ok(()));
        let mut wait = |at: Duration| rates.admit(&plugin("user-a-b"), post(), start + at);
        assert_eq!(
            wait(Duration::from_secs(3)),
            Err(HostError::RateLimited {
                retry_after_ms: 7000
            })
        );
        assert_eq!(wait(Duration::from_secs(10)), Ok(()));
    }

    #[test]
    fn each_plugin_has_its_own_budget_and_forgetting_resets_it() {
        let mut rates = RateLimits::default();
        let now = Instant::now();
        assert_eq!(rates.admit(&plugin("user-a-b"), post(), now), Ok(()));
        assert_eq!(rates.admit(&plugin("user-a-c"), post(), now), Ok(()));
        assert!(rates.admit(&plugin("user-a-b"), post(), now).is_err());
        rates.forget(&plugin("user-a-b"));
        assert_eq!(rates.admit(&plugin("user-a-b"), post(), now), Ok(()));
    }
}

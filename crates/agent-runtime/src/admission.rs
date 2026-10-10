//! Which idle connections to release: a pure policy over what the runtime
//! observed, so the decision is testable without processes or timers.
use nocterm_ai::AgentSessionSettings;
use std::time::{Duration, Instant};

/// A connection whose chats have no work that closing it would interrupt.
#[derive(Clone, Copy, Debug)]
pub(crate) struct IdleConnection<K> {
    key: K,
    /// When the connection last had work.
    since: Instant,
    /// A panel shows one of its chats.
    shown: bool,
}

/// The idle connections to release, oldest first.
///
/// A connection whose chat is shown outlives `idle_timeout_secs` and
/// `max_idle`: the user is looking at it and the most likely to write next,
/// and starting an agent is slow. The others are released after the timeout,
/// beyond the limit, and the oldest when a waiting chat needs its slot. A
/// message the user sent outranks a warm session, so a shown connection
/// gives its slot up when no other idle one can.
pub(crate) fn releases<K: Copy>(
    idle: &[IdleConnection<K>],
    policy: &AgentSessionSettings,
    now: Instant,
    needs_slot: bool,
) -> Vec<K> {
    let mut idle = idle.iter().collect::<Vec<_>>();
    idle.sort_by_key(|connection| (connection.shown, connection.since));
    let hidden = idle.iter().filter(|connection| !connection.shown).count();
    let excess = hidden.saturating_sub(policy.max_idle);
    let timeout = Duration::from_secs(policy.idle_timeout_secs);
    idle.into_iter()
        .enumerate()
        .filter(|(index, connection)| {
            (needs_slot && *index == 0)
                || (!connection.shown
                    && (*index < excess || now.duration_since(connection.since) >= timeout))
        })
        .map(|(_, connection)| connection.key)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(max_idle: usize, idle_timeout_secs: u64) -> AgentSessionSettings {
        AgentSessionSettings {
            max_live: 8,
            max_idle,
            idle_timeout_secs,
            ..Default::default()
        }
    }
    fn idle(key: u32, age: u64, shown: bool, now: Instant) -> IdleConnection<u32> {
        IdleConnection {
            key,
            since: now - Duration::from_secs(age),
            shown,
        }
    }

    #[test]
    fn the_oldest_beyond_the_limit_and_the_expired_are_released() {
        let now = Instant::now() + Duration::from_secs(1000);
        let connections = [
            idle(1, 10, false, now),
            idle(2, 30, false, now),
            idle(3, 20, false, now),
        ];
        assert_eq!(releases(&connections, &policy(2, 600), now, false), [2]);
        assert_eq!(releases(&connections, &policy(3, 15), now, false), [2, 3]);
        assert_eq!(releases(&connections, &policy(3, 600), now, true), [2]);
        assert!(releases(&connections, &policy(3, 600), now, false).is_empty());
    }

    #[test]
    fn a_shown_chat_keeps_its_connection() {
        let now = Instant::now() + Duration::from_secs(1000);
        let connections = [idle(1, 900, true, now), idle(2, 5, false, now)];
        assert_eq!(releases(&connections, &policy(0, 60), now, true), [2]);
        assert!(releases(&connections[..1], &policy(0, 1), now, false).is_empty());
    }

    #[test]
    fn a_waiting_message_takes_the_slot_of_a_shown_chat_last() {
        let now = Instant::now() + Duration::from_secs(1000);
        let connections = [idle(1, 5, true, now), idle(2, 900, true, now)];
        assert_eq!(releases(&connections, &policy(0, 60), now, true), [2]);
        assert!(releases(&connections, &policy(0, 60), now, false).is_empty());
    }
}

/// Immutable observations supplied by the coordinator; admission itself reads no App.
pub(super) struct ConnectionObservation {
    key: u64,
    busy: bool,
    shown: bool,
}
impl ConnectionObservation {
    pub(super) fn new(key: u64, busy: bool, shown: bool) -> Self {
        Self { key, busy, shown }
    }
}
#[derive(Default)]
struct AdmissionPolicy {
    idle: std::collections::HashMap<u64, Instant>,
    yielding: std::collections::HashSet<u64>,
    stalled: std::collections::HashSet<u64>,
}
#[derive(Default)]
pub(super) struct AdmissionState {
    pending: std::collections::VecDeque<super::Client>,
    policy: AdmissionPolicy,
    maintenance: Option<gpui::Task<()>>,
}
impl AdmissionState {
    pub(super) fn request(&mut self, client: super::Client) {
        if !self
            .pending
            .iter()
            .any(|pending| pending.id() == client.id())
        {
            self.pending.push_back(client)
        }
    }
    pub(super) fn withdraw(&mut self, id: gpui::EntityId) {
        self.pending.retain(|client| client.id() != id)
    }
    pub(super) fn pending(&self) -> impl Iterator<Item = &super::Client> {
        self.pending.iter()
    }
    pub(super) fn pending_len(&self) -> usize {
        self.pending.len()
    }
    pub(super) fn pop(&mut self) -> Option<super::Client> {
        self.pending.pop_front()
    }
    pub(super) fn defer(&mut self, client: super::Client) {
        self.pending.push_back(client)
    }
    pub(super) fn clear(&mut self) {
        self.pending.clear()
    }
    pub(super) fn running(&self) -> bool {
        self.maintenance.is_some()
    }
    pub(super) fn install(&mut self, task: gpui::Task<()>) {
        self.maintenance = Some(task)
    }
    pub(super) fn stop(&mut self) {
        self.pending.clear();
        self.maintenance.take();
    }
    pub(super) fn idle_task(&mut self) {
        self.maintenance.take();
    }
    pub(super) fn released(&mut self, key: u64) {
        self.policy.released(key);
    }
    pub(super) fn failed_close(&mut self, key: u64) {
        self.policy.failed_close(key);
    }
    pub(super) fn decisions(
        &mut self,
        observations: &[ConnectionObservation],
        closing: &[u64],
        policy: &AgentSessionSettings,
        now: Instant,
        waiting: usize,
    ) -> Vec<u64> {
        self.policy
            .decisions(observations, closing, policy, now, waiting)
    }
}

impl AdmissionPolicy {
    pub(super) fn released(&mut self, key: u64) {
        self.idle.remove(&key);
        self.yielding.remove(&key);
    }
    pub(super) fn failed_close(&mut self, key: u64) {
        self.stalled.insert(key);
    }
    pub(super) fn decisions(
        &mut self,
        observations: &[ConnectionObservation],
        closing: &[u64],
        policy: &AgentSessionSettings,
        now: Instant,
        waiting: usize,
    ) -> Vec<u64> {
        self.yielding.retain(|key| {
            observations
                .iter()
                .any(|observation| observation.key == *key)
        });
        let mut idle = Vec::new();
        for observation in observations {
            if self.yielding.contains(&observation.key) {
                continue;
            }
            if observation.busy {
                self.idle.remove(&observation.key);
            } else {
                idle.push(IdleConnection {
                    key: observation.key,
                    since: *self.idle.entry(observation.key).or_insert(now),
                    shown: observation.shown,
                });
            }
        }
        let freeing = self.yielding.len()
            + closing
                .iter()
                .filter(|key| !self.stalled.contains(key))
                .count();
        let needs_slot = waiting > freeing && observations.len() + closing.len() >= policy.max_live;
        let decisions = releases(&idle, policy, now, needs_slot);
        self.yielding.extend(decisions.iter().copied());
        decisions
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    fn policy() -> AgentSessionSettings {
        AgentSessionSettings {
            max_live: 2,
            max_idle: 2,
            idle_timeout_secs: 60,
            ..Default::default()
        }
    }
    #[test]
    fn yielding_and_confirmed_cleanup_reserve_only_the_slot_already_returning() {
        let now = Instant::now();
        let mut admission = AdmissionPolicy::default();
        let observations = [
            ConnectionObservation {
                key: 1,
                busy: false,
                shown: false,
            },
            ConnectionObservation {
                key: 2,
                busy: false,
                shown: false,
            },
        ];
        assert_eq!(
            admission.decisions(&observations, &[], &policy(), now, 1),
            [1]
        );
        assert!(
            admission
                .decisions(&observations, &[], &policy(), now, 1)
                .is_empty()
        );
        admission.released(1);
        assert!(
            admission
                .decisions(&observations[1..], &[1], &policy(), now, 1)
                .is_empty()
        );
        admission.failed_close(1);
        assert_eq!(
            admission.decisions(&observations[1..], &[1], &policy(), now, 1),
            [2]
        );
    }
    #[test]
    fn busy_work_resets_idle_age_before_a_later_timeout() {
        let now = Instant::now();
        let mut admission = AdmissionPolicy::default();
        let hidden = [ConnectionObservation {
            key: 1,
            busy: false,
            shown: false,
        }];
        assert!(
            admission
                .decisions(&hidden, &[], &policy(), now, 0)
                .is_empty()
        );
        assert!(
            admission
                .decisions(
                    &[ConnectionObservation {
                        key: 1,
                        busy: true,
                        shown: false
                    }],
                    &[],
                    &policy(),
                    now + Duration::from_secs(59),
                    0
                )
                .is_empty()
        );
        assert!(
            admission
                .decisions(&hidden, &[], &policy(), now + Duration::from_secs(60), 0)
                .is_empty()
        );
        assert_eq!(
            admission.decisions(&hidden, &[], &policy(), now + Duration::from_secs(120), 0),
            [1]
        );
    }
}

//! Which idle connections to release: a pure policy over what the runtime
//! observed, so the decision is testable without processes or timers.
use nocterm_ai::AgentSessionSettings;
use std::time::{Duration, Instant};

/// A connection whose chats have no work that closing it would interrupt.
#[derive(Clone, Copy, Debug)]
pub(crate) struct IdleConnection<K> {
    pub key: K,
    /// When the connection last had work.
    pub since: Instant,
    /// A panel shows one of its chats.
    pub shown: bool,
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

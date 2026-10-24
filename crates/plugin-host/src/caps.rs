//! The host-wide caps on user plugins (design 5.12).

use std::time::Duration;

/// Most user plugins enabled at once.
pub const MAX_ENABLED: usize = 16;
/// Most renders per second across all user plugins; `AfterMs` refreshes count.
pub const MAX_RENDERS_PER_SECOND: usize = 32;
/// Traps and missed deadlines in one session that disable a plugin.
pub const MAX_FAULTS: u8 = 3;
/// How long a gesture token stays valid.
pub const GESTURE_TTL: Duration = Duration::from_secs(1);
/// How often a plugin's observations are sampled.
pub const OBSERVE_EVERY: Duration = Duration::from_secs(1);
/// The same for a plugin whose panel has focus.
pub const OBSERVE_EVERY_FOCUSED: Duration = Duration::from_millis(250);
/// Most outcomes kept for a plugin that has not asked since.
pub const MAX_OUTCOMES: usize = 32;

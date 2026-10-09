//! Wall-clock time as saved chats and transcripts record it.
//!
//! A leaf module: the thread model and saved history both depend on it, so
//! neither depends on the other for the clock.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

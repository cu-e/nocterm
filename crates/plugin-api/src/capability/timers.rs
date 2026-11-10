//! `timers.set`: a `timer` event after a delay, under the effect's key.

use serde::{Deserialize, Serialize};

use super::{Capability, sealed};
use crate::LimitError;

/// The shortest delay, in milliseconds.
pub const MIN_TIMER_MS: u32 = 1000;
/// The longest delay, in milliseconds.
pub const MAX_TIMER_MS: u32 = 3_600_000;
/// Most timers one plugin has waiting; setting a key that waits replaces it.
pub const MAX_TIMERS: usize = 8;

/// Wakes the plugin with a `timer` event after `after_ms`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SetTimer {
    pub after_ms: u32,
}

impl SetTimer {
    pub fn check(&self) -> Result<(), LimitError> {
        if self.after_ms < MIN_TIMER_MS {
            return Err(LimitError::Syntax { field: "after_ms" });
        }
        if self.after_ms > MAX_TIMER_MS {
            return Err(LimitError::Range {
                field: "after_ms",
                max: MAX_TIMER_MS,
            });
        }
        Ok(())
    }
}

/// Marker of `timers.set`.
pub struct Timer;

impl sealed::Sealed for Timer {}

impl Capability for Timer {
    const ID: &'static str = "timers.set";
    type Value = SetTimer;
}

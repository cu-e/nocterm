//! The local wall clock, read from the operating system.

use chrono::{Local, Offset as _};
use nocterm_core::clock::{WallClock, WallTime};

/// Reads the system time and the local time zone, so the zone is the one in effect at that
/// instant (daylight saving included). A zone the system cannot report is UTC.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl WallClock for SystemClock {
    fn now(&self) -> WallTime {
        let now = Local::now();
        WallTime {
            unix_seconds: now.timestamp(),
            utc_offset_seconds: now.offset().fix().local_minus_utc(),
        }
    }
}

#[cfg(test)]
mod tests;

//! The wall clock as a contract: the local date and time are read through [`WallClock`], never
//! from the system by a consumer.
//!
//! [`WallTime`] holds only integers, so no calendar or time-zone library is needed here. The
//! native reading is an adapter's (`nocterm-local`); `FixedClock` serves tests.

#[cfg(any(test, feature = "test-support"))]
use std::sync::{Mutex, PoisonError};

/// An instant on the wall clock: UTC seconds and the offset of the local time zone then.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WallTime {
    /// Seconds since 1970-01-01 00:00:00 UTC.
    pub unix_seconds: i64,
    /// Seconds the local time is ahead of UTC; negative west of Greenwich.
    pub utc_offset_seconds: i32,
}

const SECONDS_PER_DAY: i64 = 86_400;

impl WallTime {
    /// Seconds since local midnight, in `0..86_400`.
    pub fn local_second_of_day(self) -> u32 {
        let local = self
            .unix_seconds
            .saturating_add(i64::from(self.utc_offset_seconds));
        local.rem_euclid(SECONDS_PER_DAY) as u32
    }

    /// The local hour (`0..24`) and minute (`0..60`).
    pub fn local_hour_minute(self) -> (u8, u8) {
        let (hour, minute, _) = self.local_hour_minute_second();
        (hour, minute)
    }

    /// The local hour (`0..24`), minute (`0..60`) and second (`0..60`).
    pub fn local_hour_minute_second(self) -> (u8, u8, u8) {
        let second = self.local_second_of_day();
        (
            (second / 3600) as u8,
            (second % 3600 / 60) as u8,
            (second % 60) as u8,
        )
    }

    /// The local calendar date as year, month (`1..=12`) and day (`1..=31`), in the proleptic
    /// Gregorian calendar.
    pub fn local_date(self) -> (i64, u8, u8) {
        let local = self
            .unix_seconds
            .saturating_add(i64::from(self.utc_offset_seconds));
        // Days since 1970-01-01 to a civil date (Howard Hinnant's `civil_from_days`).
        let z = local.div_euclid(SECONDS_PER_DAY) + 719_468;
        let era = z.div_euclid(146_097);
        let day_of_era = z.rem_euclid(146_097);
        let year_of_era =
            (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let shifted_month = (5 * day_of_year + 2) / 153;
        let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u8;
        let month = if shifted_month < 10 {
            shifted_month + 3
        } else {
            shifted_month - 9
        } as u8;
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        (year, month, day)
    }

    /// Seconds until the local minute changes, in `1..=60`.
    pub fn seconds_to_next_minute(self) -> u8 {
        60 - (self.local_second_of_day() % 60) as u8
    }
}

/// Reads the wall clock.
pub trait WallClock: Send + Sync + 'static {
    /// The current instant. Infallible: an adapter that cannot read the time zone reports UTC.
    fn now(&self) -> WallTime;
}

/// A clock a test sets and moves.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default)]
pub struct FixedClock {
    time: Mutex<WallTime>,
    reads: std::sync::atomic::AtomicUsize,
}

#[cfg(any(test, feature = "test-support"))]
impl FixedClock {
    pub fn new(time: WallTime) -> Self {
        Self {
            time: Mutex::new(time),
            reads: Default::default(),
        }
    }

    /// Moves the clock to `time`.
    pub fn set(&self, time: WallTime) {
        *self.time.lock().unwrap_or_else(PoisonError::into_inner) = time;
    }

    /// How many times the clock was read.
    pub fn reads(&self) -> usize {
        self.reads.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl WallClock for FixedClock {
    fn now(&self) -> WallTime {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *self.time.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests;

//! The `[clock]` settings section: how the Clock widget writes the time.

use std::time::Duration;

use nocterm_core::clock::WallTime;
use nocterm_settings::SettingsSection;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The longest the clock waits before it reads the time again.
const MAX_WAIT: Duration = Duration::from_secs(5);

/// How the clock shows the hour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClockFormat {
    /// `13:05`.
    #[default]
    Hour24,
    /// `1:05 PM`.
    Hour12,
}

impl ClockFormat {
    /// The option key of the schema field, equal to the serde name.
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Hour24 => "hour24",
            Self::Hour12 => "hour12",
        }
    }

    pub(super) fn from_key(key: &str) -> Option<Self> {
        [Self::Hour24, Self::Hour12]
            .into_iter()
            .find(|format| format.key() == key)
    }
}

/// How the Clock widget shows the local time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ClockSettings {
    /// `hour24` writes `13:05`; `hour12` writes `1:05 PM`.
    pub format: ClockFormat,
    /// Whether the time includes seconds. The clock then updates every second instead of every
    /// minute.
    pub seconds: bool,
    /// Whether a short date (`Oct 24`) comes before the time.
    pub date: bool,
}

impl SettingsSection for ClockSettings {
    const KEY: &'static str = "clock";
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

impl ClockSettings {
    /// The text for `time`: `09:05` by default, `Oct 24 9:05:07 PM` with every option on.
    pub fn render(&self, time: WallTime) -> String {
        let (hour, minute, second) = time.local_hour_minute_second();
        let mut text = String::new();
        if self.date {
            let (_, month, day) = time.local_date();
            text.push_str(&format!(
                "{} {day} ",
                MONTHS[usize::from(month.clamp(1, 12)) - 1]
            ));
        }
        match self.format {
            ClockFormat::Hour24 => text.push_str(&format!("{hour:02}:{minute:02}")),
            ClockFormat::Hour12 => text.push_str(&format!("{}:{minute:02}", hour_on_dial(hour))),
        }
        if self.seconds {
            text.push_str(&format!(":{second:02}"));
        }
        if self.format == ClockFormat::Hour12 {
            text.push_str(if hour < 12 { " AM" } else { " PM" });
        }
        text
    }

    /// How long the text of `time` stays right: to the next second with seconds on, else to the
    /// next minute, and at most five seconds.
    ///
    /// The timer runs on the monotonic clock, which stands still while the machine sleeps and
    /// knows nothing of a time-zone change or a clock step. Reading again every few seconds
    /// shows the right time soon after any of them, and costs a read that changes nothing
    /// until the text does.
    pub fn wait(&self, time: WallTime) -> Duration {
        let to_change = if self.seconds {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(time.seconds_to_next_minute().into())
        };
        to_change.min(MAX_WAIT)
    }
}

/// The hour on a 12-hour dial: midnight and noon are 12.
fn hour_on_dial(hour: u8) -> u8 {
    match hour % 12 {
        0 => 12,
        other => other,
    }
}

#[cfg(test)]
mod tests;

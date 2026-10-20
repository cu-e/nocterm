//! Claude's account usage grammar, separate from its native request.
use nocterm_ai::usage::{RateLimit, Window};
use serde_json::Value;

/// The plan limits in Claude's account usage response, if it reports any.
pub(crate) fn claude_limits(body: &Value) -> Option<Vec<RateLimit>> {
    let windows = [
        ("five_hour", Window::FiveHour),
        ("seven_day", Window::Weekly),
        ("seven_day_opus", Window::WeeklyModel("Opus".into())),
        ("seven_day_sonnet", Window::WeeklyModel("Sonnet".into())),
    ];
    let mut limits: Vec<_> = windows
        .into_iter()
        .filter_map(|(key, window)| limit(&body[key], window))
        .collect();
    let extra = &body["extra_usage"];
    if extra["is_enabled"].as_bool() == Some(true) {
        limits.extend(limit(extra, Window::Extra));
    }
    (!limits.is_empty()).then_some(limits)
}

/// One window, whose utilization is a percentage.
fn limit(window: &Value, kind: Window) -> Option<RateLimit> {
    let used = (window["utilization"].as_f64()? / 100.).clamp(0., 1.);
    Some(RateLimit {
        window: kind,
        used: Some(used),
        resets_at: window["resets_at"].as_str().and_then(unix_seconds),
        reached: used >= 1.,
    })
}

/// An RFC 3339 timestamp (`2026-10-20T21:49:59.785225+00:00`) in seconds
/// since the Unix epoch.
fn unix_seconds(text: &str) -> Option<u64> {
    let (date, rest) = text.split_once('T')?;
    let mut date = date.splitn(3, '-').map(str::parse::<i64>);
    let (year, month, day) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let zone = rest.find(['Z', 'z', '+', '-'])?;
    let (time, offset) = rest.split_at(zone);
    let mut time = time.split(':');
    let hour: i64 = time.next()?.parse().ok()?;
    let minute: i64 = time.next()?.parse().ok()?;
    let second: i64 = time.next()?.split('.').next()?.parse().ok()?;
    let offset = match offset.as_bytes().first()? {
        b'Z' | b'z' => 0,
        sign => {
            let (hours, minutes) = offset[1..].split_once(':')?;
            let minutes = hours.parse::<i64>().ok()? * 60 + minutes.parse::<i64>().ok()?;
            if *sign == b'-' { -minutes } else { minutes }
        }
    };
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
        - offset * 60;
    u64::try_from(seconds).ok()
}

/// Days from 1970-01-01 to a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::{claude_limits, unix_seconds};
    use nocterm_ai::usage::Window;

    #[test]
    fn account_usage_reports_each_window_as_a_share() {
        let body = serde_json::json!({
            "five_hour": {"utilization": 10.0, "resets_at": "2026-10-20T21:49:59.785225+00:00"},
            "seven_day": {"utilization": 100.0, "resets_at": "2026-10-23T15:59:59.785245+00:00"},
            "seven_day_opus": null,
            "seven_day_sonnet": {"utilization": 3.0, "resets_at": null},
            "extra_usage": {"is_enabled": false, "utilization": null},
        });
        let limits = claude_limits(&body).unwrap();
        let windows: Vec<_> = limits.iter().map(|limit| limit.window.clone()).collect();
        assert_eq!(
            windows,
            [
                Window::FiveHour,
                Window::Weekly,
                Window::WeeklyModel("Sonnet".into())
            ]
        );
        assert_eq!(limits[0].used, Some(0.1));
        assert_eq!(limits[0].resets_at, Some(1_792_532_999));
        assert!(!limits[0].reached);
        assert!(limits[1].reached);
        assert_eq!(limits[2].resets_at, None);
        assert!(claude_limits(&serde_json::json!({"five_hour": null})).is_none());
    }

    #[test]
    fn timestamps_honour_their_offset() {
        assert_eq!(unix_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_seconds("1970-01-01T01:00:00+01:00"), Some(0));
        assert_eq!(
            unix_seconds("2000-03-01T00:00:00.5-00:30"),
            Some(951_870_600)
        );
        assert_eq!(unix_seconds("not a time"), None);
    }
}

use std::time::Duration;

use nocterm_core::clock::WallTime;

use super::{ClockFormat, ClockSettings, MAX_WAIT};

/// 2026-10-24 21:05:07 UTC.
const EVENING: i64 = 1_792_884_600 - 2 * 3600 - 25 * 60 + 7;

fn at(unix_seconds: i64) -> WallTime {
    WallTime {
        unix_seconds,
        utc_offset_seconds: 0,
    }
}

fn with(format: ClockFormat, seconds: bool, date: bool) -> ClockSettings {
    ClockSettings {
        format,
        seconds,
        date,
    }
}

#[test]
fn the_default_is_hours_and_minutes_on_a_24_hour_clock() {
    assert_eq!(ClockSettings::default().render(at(EVENING)), "21:05");
    assert_eq!(
        ClockSettings::default().render(at(9 * 3600 + 5 * 60)),
        "09:05"
    );
}

#[test]
fn the_12_hour_clock_writes_am_and_pm_and_names_midnight_and_noon_12() {
    let twelve = with(ClockFormat::Hour12, false, false);
    assert_eq!(twelve.render(at(EVENING)), "9:05 PM");
    assert_eq!(twelve.render(at(9 * 3600 + 5 * 60)), "9:05 AM");
    assert_eq!(twelve.render(at(5 * 60)), "12:05 AM");
    assert_eq!(twelve.render(at(12 * 3600)), "12:00 PM");
    assert_eq!(twelve.render(at(23 * 3600 + 59 * 60)), "11:59 PM");
}

#[test]
fn seconds_follow_the_minutes_and_come_before_the_meridiem() {
    assert_eq!(
        with(ClockFormat::Hour24, true, false).render(at(EVENING)),
        "21:05:07"
    );
    assert_eq!(
        with(ClockFormat::Hour12, true, false).render(at(EVENING)),
        "9:05:07 PM"
    );
}

#[test]
fn the_date_is_short_and_comes_first() {
    assert_eq!(
        with(ClockFormat::Hour24, false, true).render(at(EVENING)),
        "Oct 24 21:05"
    );
    assert_eq!(
        with(ClockFormat::Hour12, true, true).render(at(EVENING)),
        "Oct 24 9:05:07 PM"
    );
    assert_eq!(
        with(ClockFormat::Hour24, false, true).render(at(0)),
        "Jan 1 00:00"
    );
}

#[test]
fn the_date_is_the_local_one_across_midnight() {
    let late = WallTime {
        unix_seconds: EVENING + 3 * 3600,
        utc_offset_seconds: 3600,
    };
    assert_eq!(
        with(ClockFormat::Hour24, false, true).render(late),
        "Oct 25 01:05"
    );
}

#[test]
fn the_text_is_valid_for_a_second_with_seconds_and_until_the_minute_ends_without() {
    let minutes = ClockSettings::default();
    assert_eq!(minutes.wait(at(EVENING + 51)), Duration::from_secs(2));
    assert_eq!(minutes.wait(at(EVENING + 56)), Duration::from_secs(5));
    let seconds = with(ClockFormat::Hour24, true, false);
    assert_eq!(seconds.wait(at(EVENING)), Duration::from_secs(1));
    assert_eq!(seconds.wait(at(60)), Duration::from_secs(1));
}

#[test]
fn the_wait_is_capped_so_a_suspend_or_a_time_jump_shows_within_seconds() {
    let minutes = ClockSettings::default();
    assert_eq!(minutes.wait(at(EVENING)), MAX_WAIT);
    assert_eq!(minutes.wait(at(60)), MAX_WAIT);
    assert_eq!(MAX_WAIT, Duration::from_secs(5));
}

#[test]
fn the_section_reads_from_its_stored_names() {
    let settings: ClockSettings =
        toml::from_str("format = \"hour12\"\nseconds = true\n").expect("a valid [clock] table");
    assert_eq!(settings, with(ClockFormat::Hour12, true, false));
    assert_eq!(
        toml::from_str::<ClockSettings>("").unwrap(),
        ClockSettings::default()
    );
}

#[test]
fn every_format_has_a_key_that_finds_it_again() {
    for format in [ClockFormat::Hour24, ClockFormat::Hour12] {
        assert_eq!(ClockFormat::from_key(format.key()), Some(format));
        let stored = toml::to_string(&ClockSettings {
            format,
            ..Default::default()
        })
        .unwrap();
        assert!(stored.contains(&format!("format = \"{}\"", format.key())));
    }
    assert_eq!(ClockFormat::from_key("hour48"), None);
}

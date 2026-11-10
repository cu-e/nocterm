use super::*;

fn time(unix_seconds: i64, utc_offset_seconds: i32) -> WallTime {
    WallTime {
        unix_seconds,
        utc_offset_seconds,
    }
}

#[test]
fn utc_midnight_is_zero_zero() {
    assert_eq!(time(0, 0).local_hour_minute(), (0, 0));
    assert_eq!(time(86_400 * 3, 0).local_hour_minute(), (0, 0));
}

#[test]
fn the_offset_moves_the_local_time_across_midnight() {
    // 23:30 UTC is 05:00 the next day at +05:30 and 18:30 at -05:00.
    let utc = 23 * 3600 + 30 * 60;
    assert_eq!(time(utc, 5 * 3600 + 1800).local_hour_minute(), (5, 0));
    assert_eq!(time(utc, -5 * 3600).local_hour_minute(), (18, 30));
}

#[test]
fn a_time_before_the_epoch_still_lands_in_the_day() {
    // 1969-12-31 23:59:59 UTC.
    assert_eq!(time(-1, 0).local_hour_minute(), (23, 59));
    assert_eq!(time(-1, 3600).local_hour_minute(), (0, 59));
}

#[test]
fn the_wait_for_the_next_minute_is_between_one_and_sixty_seconds() {
    assert_eq!(time(0, 0).seconds_to_next_minute(), 60);
    assert_eq!(time(59, 0).seconds_to_next_minute(), 1);
    assert_eq!(time(61, 0).seconds_to_next_minute(), 59);
    assert_eq!(time(10, 45).seconds_to_next_minute(), 5);
}

#[test]
fn the_local_second_is_part_of_the_time_of_day() {
    assert_eq!(time(0, 0).local_hour_minute_second(), (0, 0, 0));
    assert_eq!(
        time(9 * 3600 + 5 * 60 + 7, 0).local_hour_minute_second(),
        (9, 5, 7)
    );
    assert_eq!(time(-1, 0).local_hour_minute_second(), (23, 59, 59));
}

#[test]
fn the_local_date_is_the_calendar_day_of_the_local_time() {
    assert_eq!(time(0, 0).local_date(), (1970, 1, 1));
    assert_eq!(time(-1, 0).local_date(), (1969, 12, 31));
    // 2024-02-29 12:00:00 UTC, a leap day.
    assert_eq!(time(1_709_208_000, 0).local_date(), (2024, 2, 29));
    // 2026-10-24 23:30:00 UTC is already the 25th at +05:00 and still the 24th at -05:00.
    let utc = 1_792_884_600;
    assert_eq!(time(utc, 0).local_date(), (2026, 10, 24));
    assert_eq!(time(utc, 5 * 3600).local_date(), (2026, 10, 25));
    assert_eq!(time(utc, -5 * 3600).local_date(), (2026, 10, 24));
    // 2000-03-01 00:00:00 UTC follows the leap day of a century year.
    assert_eq!(time(951_868_800, 0).local_date(), (2000, 3, 1));
    assert_eq!(time(951_868_799, 0).local_date(), (2000, 2, 29));
}

#[test]
fn extreme_values_do_not_overflow() {
    for unix_seconds in [i64::MIN, i64::MAX] {
        let time = time(unix_seconds, i32::MAX);
        let (hour, minute) = time.local_hour_minute();
        assert!(hour < 24 && minute < 60);
        let (_, month, day) = time.local_date();
        assert!((1..=12).contains(&month) && (1..=31).contains(&day));
    }
}

#[test]
fn a_fixed_clock_reports_what_was_set_and_counts_reads() {
    let clock = FixedClock::new(time(60, 0));
    assert_eq!(clock.now(), time(60, 0));
    clock.set(time(120, 3600));
    assert_eq!(clock.now(), time(120, 3600));
    assert_eq!(clock.reads(), 2);
}

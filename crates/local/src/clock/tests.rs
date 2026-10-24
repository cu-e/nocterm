use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

const LARGEST_OFFSET: i32 = 14 * 3600;

#[test]
fn the_offset_is_a_real_time_zone() {
    let time = SystemClock.now();
    assert!(
        (-LARGEST_OFFSET..=LARGEST_OFFSET).contains(&time.utc_offset_seconds),
        "offset {}",
        time.utc_offset_seconds
    );
}

#[test]
fn the_time_follows_the_system_clock() {
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let time = SystemClock.now();
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert!((before..=after + 1).contains(&time.unix_seconds));
}

#[test]
fn the_local_time_is_of_the_day() {
    let (hour, minute) = SystemClock.now().local_hour_minute();
    assert!(hour < 24 && minute < 60);
}

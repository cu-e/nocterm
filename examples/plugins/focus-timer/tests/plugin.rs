//! Behaviour of the focus timer, driven through the SDK's native test host.

use focus_timer::FocusTimer;
use nocterm_plugin_sdk::{
    api::{ClockNow, Event, HostError, NodeKind},
    observe::Clock,
    testing::Host,
};

const MANIFEST: &str = include_str!("../plugin.toml");

fn clock(unix_minutes: i64) -> ClockNow {
    ClockNow {
        unix_minutes,
        utc_offset_seconds: 0,
    }
}

fn host() -> Host {
    Host::new(MANIFEST).expect("the manifest is a valid code package")
}

fn press(host: &mut Host, local: &str, key: &str) -> Vec<String> {
    let effects = host
        .event(&mut FocusTimer, local, Event::Activated { key: key.into() })
        .expect("the event is answered");
    effects
        .0
        .iter()
        .map(|request| format!("{:?}", request.effect))
        .collect()
}

fn texts(view: &nocterm_plugin_sdk::api::View) -> Vec<String> {
    view.nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Text(text) => Some(text.clone()),
            NodeKind::Button(label) => Some(label.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_bar_shows_the_minutes_since_the_stored_start() {
    let mut host = host();
    host.observe::<Clock>(clock(1_000));
    let idle = host.render(&mut FocusTimer, "bar").unwrap();
    assert_eq!(texts(&idle), ["Focus"]);

    host.store("start", "988");
    let running = host.render(&mut FocusTimer, "bar").unwrap();
    assert_eq!(texts(&running), ["Focus 12m"]);
}

#[test]
fn activating_the_bar_starts_a_round_and_opens_the_panel() {
    let mut host = host();
    host.observe::<Clock>(clock(1_000));

    let effects = press(&mut host, "bar", "open");

    assert_eq!(effects.len(), 3, "{effects:?}");
    assert!(effects[0].contains("StorageSet"));
    assert!(effects[1].contains("PanelsSetOpen"));
    assert!(effects[2].contains("PanelsSetBadge"));
    assert_eq!(
        host.storage().get("start").map(String::as_str),
        Some("1000")
    );
}

#[test]
fn pressing_the_bar_keeps_the_finished_rounds_on_the_badge() {
    let mut host = host();
    host.observe::<Clock>(clock(1_000)).store("rounds", "2");

    let effects = press(&mut host, "bar", "open");

    let badge = effects
        .iter()
        .find(|effect| effect.contains("PanelsSetBadge"))
        .expect("the badge is set");
    assert!(badge.contains("Some(2)"), "{badge}");

    let mut fresh = self::host();
    fresh.observe::<Clock>(clock(1_000));
    let effects = press(&mut fresh, "bar", "open");
    let badge = effects
        .iter()
        .find(|effect| effect.contains("PanelsSetBadge"))
        .expect("the badge is set");
    assert!(
        badge.contains("None"),
        "no finished round, no badge: {badge}"
    );
}

#[test]
fn the_panel_meter_follows_the_goal_setting() {
    let mut host = host();
    host.observe::<Clock>(clock(1_010)).store("start", "1000");

    let view = host.render(&mut FocusTimer, "panel").unwrap();

    assert_eq!(texts(&view)[0], "10 of 25 min");
    assert!(view.nodes.iter().any(|n| n.kind == NodeKind::Meter(40)));
}

#[test]
fn resetting_after_the_goal_counts_a_finished_round() {
    let mut host = host();
    host.observe::<Clock>(clock(1_030)).store("start", "1000");

    let effects = press(&mut host, "panel", "reset");

    assert!(
        effects.iter().any(|e| e.contains("PanelsSetBadge")),
        "{effects:?}"
    );
    assert_eq!(host.storage().get("rounds").map(String::as_str), Some("1"));
    assert_eq!(
        host.storage().get("start").map(String::as_str),
        Some("1030")
    );
}

#[test]
fn resetting_before_the_goal_restarts_without_counting() {
    let mut host = host();
    host.observe::<Clock>(clock(1_005)).store("start", "1000");

    press(&mut host, "panel", "reset");

    assert_eq!(host.storage().get("rounds"), None);
    assert_eq!(
        host.storage().get("start").map(String::as_str),
        Some("1005")
    );
}

#[test]
fn closing_the_panel_asks_the_host_to_close_it() {
    let mut host = host();
    let effects = press(&mut host, "panel", "close");
    assert_eq!(effects.len(), 1);
    assert!(effects[0].contains("PanelsSetOpen"));
}

#[test]
fn a_plugin_without_the_clock_still_draws() {
    let mut host = host();
    host.observe_error::<Clock>(HostError::Unavailable);
    let view = host.render(&mut FocusTimer, "panel").unwrap();
    assert!(texts(&view)[0].starts_with("No round yet"));
}

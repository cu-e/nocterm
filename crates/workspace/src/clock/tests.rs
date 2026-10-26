use std::{sync::Arc, time::Duration};

use gpui_kit::{AppContext as _, Entity, TestAppContext};
use nocterm_core::clock::{FixedClock, WallTime};
use nocterm_store::{SettingsConfig, SettingsHandle};

use super::{ClockFormat, ClockSettings, PLUGIN, TIME, view::ClockView};

const NINE_FIVE: i64 = 9 * 3600 + 5 * 60;

fn clock_at(unix_seconds: i64, utc_offset_seconds: i32) -> Arc<FixedClock> {
    Arc::new(FixedClock::new(WallTime {
        unix_seconds,
        utc_offset_seconds,
    }))
}

fn settings(cx: &mut TestAppContext) -> SettingsHandle {
    cx.update(|cx| {
        let settings = SettingsConfig::in_memory(Default::default()).install(cx);
        settings.register::<ClockSettings>(cx);
        settings
    })
}

fn view(clock: &Arc<FixedClock>, cx: &mut TestAppContext) -> Entity<ClockView> {
    view_over(clock, &settings(cx), cx)
}

fn view_over(
    clock: &Arc<FixedClock>,
    settings: &SettingsHandle,
    cx: &mut TestAppContext,
) -> Entity<ClockView> {
    let (clock, settings) = (clock.clone(), settings.clone());
    cx.update(|cx| cx.new(|cx| ClockView::new(clock, settings, cx)))
}

fn restyle(settings: &SettingsHandle, style: ClockSettings, cx: &mut TestAppContext) {
    cx.update(|cx| settings.update::<ClockSettings>(cx, move |section| *section = style))
        .detach();
    cx.run_until_parked();
}

/// How many times the clock is read over `seconds` of waiting.
fn reads_over(clock: &FixedClock, seconds: u64, cx: &mut TestAppContext) -> usize {
    let before = clock.reads();
    for _ in 0..seconds {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }
    clock.reads() - before
}

fn text(view: &Entity<ClockView>, cx: &mut TestAppContext) -> String {
    cx.update(|cx| view.read(cx).text())
}

#[test]
fn the_plugin_owns_one_widget_that_the_default_layout_leaves_in_the_tray() {
    assert_eq!(PLUGIN.id.as_str(), "clock");
    assert!(!PLUGIN.required);
    assert!(PLUGIN.settings_page.is_none());
    assert_eq!(PLUGIN.contributions.len(), 1);
    assert_eq!(PLUGIN.contributions[0].id, TIME.id);
    assert_eq!(TIME.placement.auto, nocterm_plugin_model::AutoPlace::Tray);
}

#[gpui_kit::test]
fn the_view_shows_hours_and_minutes_with_two_digits(cx: &mut TestAppContext) {
    let clock = clock_at(NINE_FIVE + 7, 0);
    let view = view(&clock, cx);
    assert_eq!(text(&view, cx), "09:05");
}

#[gpui_kit::test]
fn the_view_shows_the_local_time_of_the_clocks_offset(cx: &mut TestAppContext) {
    let clock = clock_at(23 * 3600 + 30 * 60, 5 * 3600 + 1800);
    let view = view(&clock, cx);
    assert_eq!(text(&view, cx), "05:00");
}

#[gpui_kit::test]
fn the_view_moves_on_when_the_minute_changes_and_not_before(cx: &mut TestAppContext) {
    let clock = clock_at(NINE_FIVE + 10, 0);
    let view = view(&clock, cx);
    cx.run_until_parked();

    clock.set(WallTime {
        unix_seconds: NINE_FIVE + 40,
        utc_offset_seconds: 0,
    });
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(text(&view, cx), "09:05", "the minute has not ended");

    clock.set(WallTime {
        unix_seconds: NINE_FIVE + 60,
        utc_offset_seconds: 0,
    });
    cx.executor().advance_clock(Duration::from_secs(40));
    cx.run_until_parked();
    assert_eq!(text(&view, cx), "09:06");
}

#[gpui_kit::test]
fn a_jump_of_the_time_shows_within_seconds_not_at_the_end_of_the_minute(cx: &mut TestAppContext) {
    // The timer has most of a minute left when the machine wakes up two hours later, or the
    // time zone changes.
    let clock = clock_at(NINE_FIVE + 5, 0);
    let view = view(&clock, cx);
    cx.run_until_parked();
    assert_eq!(text(&view, cx), "09:05");

    clock.set(WallTime {
        unix_seconds: NINE_FIVE + 2 * 3600 + 5,
        utc_offset_seconds: 0,
    });
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();

    assert_eq!(text(&view, cx), "11:05");
}

#[gpui_kit::test]
fn the_timer_stops_when_the_view_is_dropped(cx: &mut TestAppContext) {
    let clock = clock_at(NINE_FIVE, 0);
    let view = view(&clock, cx);
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(180));
    cx.run_until_parked();
    let while_alive = clock.reads();
    assert!(while_alive > 2, "the view keeps reading the clock");

    drop(view);
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(600));
    cx.run_until_parked();
    assert_eq!(clock.reads(), while_alive);
}

#[test]
fn the_plugin_stores_its_settings_in_the_clock_section() {
    use nocterm_settings::SettingsSection as _;
    assert_eq!(PLUGIN.settings_section, Some(ClockSettings::KEY));
    assert!(
        PLUGIN
            .settings
            .groups
            .iter()
            .all(|group| group.contribution.as_ref() == Some(&TIME.id)),
        "every field belongs to the clock widget"
    );
}

#[gpui_kit::test]
fn changing_the_settings_changes_the_text_at_once_and_the_view_keeps_following(
    cx: &mut TestAppContext,
) {
    let clock = clock_at(NINE_FIVE + 7, 0);
    let settings = settings(cx);
    let view = view_over(&clock, &settings, cx);
    assert_eq!(text(&view, cx), "09:05");

    restyle(
        &settings,
        ClockSettings {
            format: ClockFormat::Hour12,
            seconds: true,
            date: true,
        },
        cx,
    );
    assert_eq!(text(&view, cx), "Jan 1 9:05:07 AM");

    restyle(&settings, ClockSettings::default(), cx);
    assert_eq!(text(&view, cx), "09:05");
}

#[gpui_kit::test]
fn the_timer_runs_per_second_with_seconds_on_and_per_minute_with_them_off(cx: &mut TestAppContext) {
    let clock = clock_at(NINE_FIVE, 0);
    let settings = settings(cx);
    let view = view_over(&clock, &settings, cx);
    cx.run_until_parked();
    assert!(
        reads_over(&clock, 30, cx) <= 7,
        "by the minute the clock is read once in five seconds at most"
    );

    restyle(
        &settings,
        ClockSettings {
            seconds: true,
            ..Default::default()
        },
        cx,
    );
    assert!(
        reads_over(&clock, 10, cx) >= 10,
        "with seconds the clock is read every second"
    );

    restyle(&settings, ClockSettings::default(), cx);
    assert!(
        reads_over(&clock, 30, cx) <= 7,
        "without seconds it goes back to the slower cadence"
    );
    drop(view);
}

#[gpui_kit::test]
fn a_change_of_the_style_replaces_the_timer_instead_of_adding_one(cx: &mut TestAppContext) {
    let clock = clock_at(NINE_FIVE, 0);
    let settings = settings(cx);
    let view = view_over(&clock, &settings, cx);
    let seconds = ClockSettings {
        seconds: true,
        ..Default::default()
    };
    for _ in 0..4 {
        restyle(&settings, seconds.clone(), cx);
        restyle(&settings, ClockSettings::default(), cx);
    }
    restyle(&settings, seconds, cx);
    let reads = reads_over(&clock, 10, cx);
    assert!(
        (10..=11).contains(&reads),
        "one timer reads once per second, not {reads} times in ten"
    );
    drop(view);
}

#[gpui_kit::test]
fn a_change_to_another_section_leaves_the_timer_alone(cx: &mut TestAppContext) {
    let clock = clock_at(NINE_FIVE, 0);
    let settings = settings(cx);
    let view = view_over(&clock, &settings, cx);
    cx.run_until_parked();
    let before = clock.reads();
    cx.update(|cx| {
        settings.register::<nocterm_window_layout::LayoutSettings>(cx);
        settings.update::<nocterm_window_layout::LayoutSettings>(cx, |layouts| {
            layouts.active = Some(nocterm_window_layout::PresetId::new("swapped"));
        })
    })
    .detach();
    cx.run_until_parked();
    assert_eq!(clock.reads(), before);
    drop(view);
}

#[gpui_kit::test]
fn the_binding_writes_each_field_of_the_clock_section(cx: &mut TestAppContext) {
    use nocterm_plugin_model::{FieldEdit, SettingValue};

    let settings = settings(cx);
    let source = super::plugin_settings(settings.clone());
    cx.update(|cx| {
        let shown = source.values(cx);
        assert_eq!(
            shown.get("format"),
            Some(&SettingValue::Choice("hour24".into()))
        );
        assert_eq!(shown.get("seconds"), Some(&SettingValue::Bool(false)));
        assert_eq!(shown.get("date"), Some(&SettingValue::Bool(false)));
        for (key, value) in [
            ("format", SettingValue::Choice("hour12".into())),
            ("seconds", SettingValue::Bool(true)),
            ("date", SettingValue::Bool(true)),
        ] {
            source.write(key, FieldEdit::Set(value), cx).detach();
        }
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<ClockSettings>(cx),
            &ClockSettings {
                format: ClockFormat::Hour12,
                seconds: true,
                date: true,
            }
        );
    });
}

#[gpui_kit::test]
fn the_binding_refuses_an_unknown_format(cx: &mut TestAppContext) {
    use nocterm_plugin_model::{FieldEdit, FieldError, SettingValue};
    use nocterm_ui::PluginSettingsError;

    let settings = settings(cx);
    let source = super::plugin_settings(settings.clone());
    let task = cx.update(|cx| {
        source.write(
            "format",
            FieldEdit::Set(SettingValue::Choice("hour48".into())),
            cx,
        )
    });
    cx.run_until_parked();
    let outcome = cx.foreground_executor().block_test(task);
    assert!(matches!(
        outcome,
        Err(PluginSettingsError::Field(FieldError::UnknownOption { .. }))
    ));
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<ClockSettings>(cx),
            &ClockSettings::default()
        )
    });
}

#[gpui_kit::test]
fn a_hand_edited_unknown_format_is_shown_as_the_default(cx: &mut TestAppContext) {
    use nocterm_plugin_model::SettingValue;

    let mut document = nocterm_settings::SettingsDocument::from_table(
        toml::from_str("[clock]\nformat = \"hour48\"\nseconds = true\n").unwrap(),
    );
    document.register::<ClockSettings>();
    let settings = cx.update(|cx| SettingsConfig::in_memory(document).install(cx));
    let source = super::plugin_settings(settings.clone());
    cx.update(|cx| {
        // The section does not read as a whole, so it runs on its defaults.
        let shown = source.values(cx);
        assert_eq!(
            shown.get("format"),
            Some(&SettingValue::Choice("hour24".into()))
        );
        assert_eq!(shown.get("seconds"), Some(&SettingValue::Bool(false)));
        assert!(!settings.section_errors(cx).is_empty());
    });
}

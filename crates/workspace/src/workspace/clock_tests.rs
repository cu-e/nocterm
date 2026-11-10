//! The Clock plugin in a window: placed only by a layout, created only while enabled, and
//! absent where the application supplied no clock.
use std::{sync::Arc, time::Duration};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Pixels, TestAppContext, WindowOptions, px, size,
};
use nocterm_core::clock::{FixedClock, WallTime};
use nocterm_plugin_enablement::PluginEnablement;
use nocterm_plugin_model::{ContributionId, PluginId};
use nocterm_settings::SettingsDocument;
use nocterm_store::{SettingsConfig, SettingsHandle};
use nocterm_window_layout::LayoutSettings;

use super::Workspace;
use crate::{
    clock::{ClockFormat, ClockSettings, PLUGIN, TIME},
    test_support::deps_over,
    workspace::tests::fixture_with_deps,
};

struct Window {
    handle: AnyWindowHandle,
    workspace: Entity<Workspace>,
    settings: SettingsHandle,
    clock: Arc<FixedClock>,
}

/// A layout that places the clock at the end of the right zone.
fn placing_the_clock() -> LayoutSettings {
    serde_json::from_value(serde_json::json!({
        "active": "clocked",
        "presets": [{
            "id": "clocked",
            "name": "Clocked",
            "open": [],
            "bar": {
                "left": [],
                "right": [{ "item": "workspace.notifications" }, { "item": "clock.time" }],
            },
        }],
    }))
    .expect("the stored layout is valid")
}

fn window(
    cx: &mut TestAppContext,
    layouts: Option<LayoutSettings>,
    clock: Option<Arc<FixedClock>>,
) -> Window {
    let shared = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = shared.clone();
    let supplied = clock.clone();
    let (handle, workspace, _) = fixture_with_deps(cx, move |layout, cx| {
        let mut document = SettingsDocument::default();
        document.register::<LayoutSettings>();
        document.register::<PluginEnablement>();
        if let Some(layouts) = layouts {
            document.set(layouts);
        }
        let settings = SettingsConfig::in_memory(document).install(cx);
        *slot.borrow_mut() = Some(settings.clone());
        let deps = deps_over(layout, &[], &[&PLUGIN]).with_settings(settings);
        match supplied {
            Some(clock) => deps.with_clock(clock),
            None => deps,
        }
    });
    let settings = shared.take().expect("the fixture built its dependencies");
    let clock = clock.unwrap_or_default();
    cx.update_window(handle, |_, window, cx| {
        window.resize(size(px(1400.), px(700.)));
        window.render_frame(cx);
    })
    .unwrap();
    Window {
        handle,
        workspace,
        settings,
        clock,
    }
}

fn shows_clock(cx: &mut TestAppContext, window: &Window) -> bool {
    cx.update_window(window.handle, |_, gpui_window, cx| {
        gpui_window.render_frame(cx);
        gpui_window.try_find("status-clock").is_some()
    })
    .unwrap()
}

fn nine_five() -> Arc<FixedClock> {
    Arc::new(FixedClock::new(WallTime {
        unix_seconds: 9 * 3600 + 5 * 60,
        utc_offset_seconds: 0,
    }))
}

fn set_disabled(cx: &mut TestAppContext, window: &Window, disabled: &[&'static str]) {
    let next = PluginEnablement {
        disabled: disabled.iter().map(|id| PluginId::new(id)).collect(),
        ..PluginEnablement::default()
    };
    let task = cx.update(|cx| {
        window
            .settings
            .update::<PluginEnablement>(cx, move |value| *value = next)
    });
    cx.run_until_parked();
    cx.foreground_executor().block_test(task).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_default_layout_does_not_show_the_clock(cx: &mut TestAppContext) {
    let window = window(cx, None, Some(nine_five()));
    cx.update(|cx| {
        let workspace = window.workspace.read(cx);
        assert!(workspace.is_registered(&TIME.id), "the widget is known");
    });
    assert!(!shows_clock(cx, &window));
    assert_eq!(
        window.clock.reads(),
        0,
        "nothing was created, so no timer runs"
    );
}

#[gpui_kit::test]
fn a_layout_that_places_the_clock_shows_it(cx: &mut TestAppContext) {
    let window = window(cx, Some(placing_the_clock()), Some(nine_five()));
    assert!(shows_clock(cx, &window));
}

#[gpui_kit::test]
fn a_window_without_a_clock_has_no_clock_widget(cx: &mut TestAppContext) {
    let window = window(cx, Some(placing_the_clock()), None);
    cx.update(|cx| {
        let workspace = window.workspace.read(cx);
        assert!(!workspace.is_registered(&ContributionId::new("clock.time")));
    });
    assert!(!shows_clock(cx, &window));
}

#[gpui_kit::test]
fn disabling_the_clock_drops_the_view_and_its_timer_and_enabling_brings_it_back(
    cx: &mut TestAppContext,
) {
    let window = window(cx, Some(placing_the_clock()), Some(nine_five()));
    assert!(shows_clock(cx, &window));
    cx.executor().advance_clock(Duration::from_secs(120));
    cx.run_until_parked();
    let while_on = window.clock.reads();
    assert!(while_on > 1, "the timer reads the clock while it is on");

    set_disabled(cx, &window, &["clock"]);
    assert!(!shows_clock(cx, &window));
    let after_disable = window.clock.reads();
    cx.executor().advance_clock(Duration::from_secs(600));
    cx.run_until_parked();
    assert_eq!(window.clock.reads(), after_disable, "the timer is gone");

    set_disabled(cx, &window, &[]);
    assert!(shows_clock(cx, &window), "enabling places it where it was");
}

#[gpui_kit::test]
fn a_clock_the_catalog_does_not_list_leaves_the_window_without_the_widget(cx: &mut TestAppContext) {
    let (_, workspace, _) = fixture_with_deps(cx, |layout, _| {
        deps_over(layout, &[], &[]).with_clock(nine_five())
    });
    cx.update(|cx| assert!(!workspace.read(cx).is_registered(&TIME.id)));
}

/// Another window over the same settings and clock, with the clock placed by the stored layout.
fn another_window(cx: &mut TestAppContext, window: &Window) -> Window {
    let (settings, clock) = (window.settings.clone(), window.clock.clone());
    let (handle, workspace) = cx.update(|cx| {
        let deps = deps_over(nocterm_ui::LayoutHandle::in_memory(cx), &[], &[&PLUGIN])
            .with_settings(settings.clone())
            .with_clock(clock.clone());
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(deps, window, cx))
        })
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.resize(size(px(1400.), px(700.)));
        window.render_frame(cx);
    })
    .unwrap();
    Window {
        handle,
        workspace,
        settings,
        clock,
    }
}

/// How wide the clock is drawn in `window`; a longer text is a wider clock.
fn clock_width(cx: &mut TestAppContext, window: &Window) -> Pixels {
    cx.update_window(window.handle, |_, gpui_window, cx| {
        gpui_window.render_frame(cx);
        gpui_window.find("status-clock").bounds().size.width
    })
    .unwrap()
}

fn write_clock(cx: &mut TestAppContext, window: &Window, style: ClockSettings) {
    let task = cx.update(|cx| {
        window
            .settings
            .update::<ClockSettings>(cx, move |value| *value = style)
    });
    cx.run_until_parked();
    cx.foreground_executor().block_test(task).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn changing_the_clock_settings_applies_live_in_every_window(cx: &mut TestAppContext) {
    let first = window(cx, Some(placing_the_clock()), Some(nine_five()));
    let second = another_window(cx, &first);
    let plain = clock_width(cx, &first);
    assert_eq!(clock_width(cx, &second), plain);

    write_clock(
        cx,
        &first,
        ClockSettings {
            format: ClockFormat::Hour12,
            seconds: true,
            date: true,
        },
    );
    assert!(clock_width(cx, &first) > plain, "the first window follows");
    assert!(clock_width(cx, &second) > plain, "so does the other one");

    write_clock(cx, &second, ClockSettings::default());
    assert_eq!(clock_width(cx, &first), plain);
    assert_eq!(clock_width(cx, &second), plain);
}

#[gpui_kit::test]
fn a_stored_clock_section_is_in_force_when_the_window_opens(cx: &mut TestAppContext) {
    let default = window(cx, Some(placing_the_clock()), Some(nine_five()));
    let plain = clock_width(cx, &default);
    let mut document = SettingsDocument::default();
    document.register::<LayoutSettings>();
    document.register::<ClockSettings>();
    document.set(placing_the_clock());
    document.set(ClockSettings {
        date: true,
        ..Default::default()
    });
    let settings = cx.update(|cx| SettingsConfig::in_memory(document).install(cx));
    let (handle, workspace, _) = fixture_with_deps(cx, move |layout, _| {
        deps_over(layout, &[], &[&PLUGIN])
            .with_settings(settings)
            .with_clock(nine_five())
    });
    cx.update_window(handle, |_, window, cx| {
        window.resize(size(px(1400.), px(700.)));
        window.render_frame(cx);
    })
    .unwrap();
    let width = cx
        .update_window(handle, |_, window, _| {
            window.find("status-clock").bounds().size.width
        })
        .unwrap();
    assert!(
        width > plain,
        "the stored date is shown: {width:?} vs {plain:?}"
    );
    drop(workspace);
}

//! Switching a plugin off in the composed application: what its feature still does, and what it
//! stops doing.
use std::time::Duration;

use gpui_kit::{AppContext as _, Entity, Focusable as _, TestAppContext};
use nocterm_plugin_enablement::PluginEnablement;
use nocterm_plugin_model::PluginId;
use nocterm_ui::notice;
use nocterm_workspace::Workspace;

use super::menu_layouts::window_on_local_host;

fn toggle_containers(
    handle: gpui_kit::AnyWindowHandle,
    workspace: &Entity<Workspace>,
    cx: &mut TestAppContext,
) -> usize {
    cx.update_window(handle, |_, window, cx| {
        window.focus(&workspace.read(cx).focus_handle(cx), cx);
        window.dispatch_action(Box::new(nocterm_containers_ui::ToggleContainers), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| notice::count(window, cx))
        .unwrap()
}

#[gpui_kit::test]
fn a_feature_action_for_a_disabled_plugin_says_the_plugin_is_off(cx: &mut TestAppContext) {
    let (handle, _, _, workspace) =
        window_on_local_host("[plugins]\ndisabled = [\"containers\"]\n", cx);
    assert_eq!(
        toggle_containers(handle, &workspace, cx),
        1,
        "one notice for the refusal"
    );
}

#[gpui_kit::test]
fn a_feature_action_for_an_enabled_plugin_posts_no_notice(cx: &mut TestAppContext) {
    let (handle, _, _, workspace) = window_on_local_host("", cx);
    assert_eq!(toggle_containers(handle, &workspace, cx), 0);
}

#[gpui_kit::test]
fn switching_the_monitor_off_while_it_samples_stops_the_sampling(cx: &mut TestAppContext) {
    let samples = |exec: &nocterm_session::testing::FakeExec| {
        exec.ran()
            .iter()
            .filter(|command| command.as_str() == "sh -s")
            .count()
    };
    let (_, exec, settings, _) = window_on_local_host("", cx);
    let running = samples(&exec);
    assert!(running > 0, "the placed monitor samples the host");
    cx.executor().advance_clock(Duration::from_secs(120));
    cx.run_until_parked();
    assert!(
        samples(&exec) > running,
        "the sampler keeps running while on"
    );

    let task = cx.update(|cx| {
        settings.update::<PluginEnablement>(cx, |section| {
            section.set_enabled(&PluginId::new("monitor"), false)
        })
    });
    cx.run_until_parked();
    cx.foreground_executor().block_test(task).unwrap();
    cx.run_until_parked();
    let stopped = samples(&exec);
    cx.executor().advance_clock(Duration::from_secs(600));
    cx.run_until_parked();
    assert_eq!(samples(&exec), stopped, "ran {:?}", exec.ran());
}

fn footer_shows_clock(handle: gpui_kit::AnyWindowHandle, cx: &mut TestAppContext) -> bool {
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find("status-clock").is_some()
    })
    .unwrap()
}

const CLOCK_AT_THE_END: &str = "[layouts]\nactive = \"p1\"\n[[layouts.presets]]\nid = \"p1\"\n\
    name = \"Clocked\"\n[layouts.presets.bar]\nright = [{ item = \"workspace.notifications\" }, \
    { item = \"clock.time\" }]\n";

#[gpui_kit::test]
fn the_composed_window_leaves_the_clock_out_until_a_layout_places_it(cx: &mut TestAppContext) {
    let (handle, ..) = window_on_local_host("", cx);
    assert!(
        !footer_shows_clock(handle, cx),
        "the default layout is unchanged"
    );
    let (handle, ..) = window_on_local_host(CLOCK_AT_THE_END, cx);
    assert!(footer_shows_clock(handle, cx));
}

#[gpui_kit::test]
fn switching_the_clock_plugin_off_removes_it_from_the_composed_window(cx: &mut TestAppContext) {
    let off = format!("{CLOCK_AT_THE_END}[plugins]\ndisabled = [\"clock\"]\n");
    let (handle, ..) = window_on_local_host(&off, cx);
    assert!(!footer_shows_clock(handle, cx));
}

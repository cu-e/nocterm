//! The windows a run leaves open come back at the next start.
use super::*;

fn restore(booted: &Booted, previous: &[Environment], cx: &mut TestAppContext) {
    cx.update(|cx| crate::windows::restore_windows(cx, booted, previous))
        .unwrap();
    cx.run_until_parked();
    for window in cx.update(|cx| cx.windows()) {
        show(window, cx);
    }
}

fn remembered(booted: &Booted, cx: &mut TestAppContext) -> Vec<Environment> {
    cx.update(|cx| booted.windowing().environments(cx))
}

fn titles(cx: &mut TestAppContext) -> Vec<String> {
    let mut titles: Vec<_> = cx
        .update(|cx| cx.windows())
        .into_iter()
        .filter_map(|window| title_of(window, cx))
        .collect();
    titles.sort();
    titles
}

fn close(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn each_remembered_environment_gets_a_window(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), &Arc::new(MockTransport::default()), cx);
    let previous = [
        Environment::Hub(hub_id()),
        Environment::Local,
        Environment::Local,
    ];

    restore(&booted, &previous, cx);

    assert_eq!(titles(cx), ["Nocterm", "Nocterm", "Nocterm \u{2014} Home"]);
    assert_eq!(remembered(&booted, cx), previous);
}

#[gpui_kit::test]
fn a_server_removed_from_the_list_is_forgotten(cx: &mut TestAppContext) {
    let settings =
        nocterm_store::SettingsConfig::in_memory(Default::default()).with(HubsSettings::default());
    let booted = boot_with(
        Some(&hub()),
        settings,
        &Arc::new(MockTransport::default()),
        cx,
    );

    restore(
        &booted,
        &[Environment::Hub(hub_id()), Environment::Local],
        cx,
    );

    assert_eq!(titles(cx), ["Nocterm"]);
    assert_eq!(remembered(&booted, cx), [Environment::Local]);
}

#[gpui_kit::test]
fn a_server_is_kept_when_hubs_did_not_start(cx: &mut TestAppContext) {
    let booted = boot_over(None, &Arc::new(MockTransport::default()), cx);

    restore(
        &booted,
        &[Environment::Hub(hub_id()), Environment::Local],
        cx,
    );

    assert_eq!(titles(cx), ["Nocterm"]);
    assert_eq!(
        remembered(&booted, cx),
        [Environment::Hub(hub_id()), Environment::Local]
    );
}

#[gpui_kit::test]
fn a_server_is_kept_when_the_hubs_section_could_not_be_read(cx: &mut TestAppContext) {
    let table: toml::Table = toml::from_str("hubs = \"not a table\"").unwrap();
    let settings = nocterm_store::SettingsConfig::in_memory(
        nocterm_settings::SettingsDocument::from_table(table),
    );
    let booted = boot_with(
        Some(&hub()),
        settings,
        &Arc::new(MockTransport::default()),
        cx,
    );

    restore(&booted, &[Environment::Hub(hub_id())], cx);

    assert_eq!(titles(cx), ["Nocterm"]);
    assert_eq!(
        remembered(&booted, cx),
        [Environment::Hub(hub_id()), Environment::Local]
    );
}

#[gpui_kit::test]
fn nothing_remembered_opens_one_local_window(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), &Arc::new(MockTransport::default()), cx);

    restore(&booted, &[], cx);

    assert_eq!(titles(cx), ["Nocterm"]);
    assert_eq!(remembered(&booted, cx), [Environment::Local]);
}

#[gpui_kit::test]
fn when_no_remembered_window_can_open_one_local_window_opens(cx: &mut TestAppContext) {
    let settings =
        nocterm_store::SettingsConfig::in_memory(Default::default()).with(HubsSettings::default());
    let booted = boot_with(
        Some(&hub()),
        settings,
        &Arc::new(MockTransport::default()),
        cx,
    );

    restore(&booted, &[Environment::Hub(hub_id())], cx);

    assert_eq!(titles(cx), ["Nocterm"]);
    assert_eq!(remembered(&booted, cx), [Environment::Local]);
}

#[gpui_kit::test]
fn switching_a_window_changes_the_environment_that_is_remembered(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), &Arc::new(MockTransport::default()), cx);
    let window = open(&booted, Environment::Local, cx);

    dispatch(window, nocterm_hubs_ui::OpenEnvironmentMenu, cx);
    click(window, ("environment-row", SERVER_ROW), cx);

    assert_eq!(remembered(&booted, cx), [Environment::Hub(hub_id())]);
}

#[gpui_kit::test]
fn a_window_the_user_closes_is_no_longer_remembered_except_the_last(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), &Arc::new(MockTransport::default()), cx);
    restore(
        &booted,
        &[Environment::Local, Environment::Hub(hub_id())],
        cx,
    );
    let (local, remote) = {
        let all = cx.update(|cx| cx.windows());
        let hub_first = title_of(all[0], cx).as_deref() != Some("Nocterm");
        if hub_first {
            (all[1], all[0])
        } else {
            (all[0], all[1])
        }
    };

    close(local, cx);
    assert_eq!(remembered(&booted, cx), [Environment::Hub(hub_id())]);

    close(remote, cx);
    assert_eq!(remembered(&booted, cx), [Environment::Hub(hub_id())]);
}

#[gpui_kit::test]
fn a_window_opened_by_the_new_window_command_is_remembered(cx: &mut TestAppContext) {
    let booted = boot_over(Some(&hub()), &Arc::new(MockTransport::default()), cx);
    let first = open(&booted, Environment::Hub(hub_id()), cx);

    dispatch(first, NewWindow, cx);

    assert_eq!(
        remembered(&booted, cx),
        [Environment::Hub(hub_id()), Environment::Local]
    );
}

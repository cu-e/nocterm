//! Retained Monitor policy, sampling lifetime and native page authority.
use super::*;
use gpui_kit::test::TestWindowExt as _;

fn another_settings(cx: &mut App) -> nocterm_store::SettingsHandle {
    let settings = nocterm_store::SettingsConfig::in_memory(SettingsDocument::default())
        .with(MonitorSettings {
            local: false,
            ..MonitorSettings::default()
        })
        .install(cx);
    let ready = nocterm_ui::init(DesignTokens::builtin(), settings.clone(), cx);
    let installed = crate::install(None, &ready, cx);
    gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
        cx.new(|cx| crate::MonitorPage::new(installed.settings().clone(), window, cx))
    })
    .unwrap();
    settings
}

#[gpui_kit::test]
fn independent_policy_cannot_redirect_an_existing_watch_and_closed_model_is_released(
    cx: &mut TestAppContext,
) {
    let host = FakeHost::new("Linux");
    let (window, workspace, monitor, installed) = fixture_with_installation(cx, Some(host.clone()));
    cx.run_until_parked();
    assert!(host.reading(0));
    let other = cx.update(another_settings);
    cx.run_until_parked();
    assert!(
        host.reading(0),
        "another owner's local=false does not cancel original watch"
    );
    cx.update(|cx| {
        installed
            .settings()
            .update::<MonitorSettings>(cx, |settings| settings.enabled = false)
            .detach()
    });
    cx.run_until_parked();
    assert_eq!(status(cx, &monitor), Status::Off);
    assert!(!host.reading(0));
    cx.update(|cx| {
        assert!(other.setting::<MonitorSettings>(cx).enabled);
        installed
            .settings()
            .update::<MonitorSettings>(cx, |settings| {
                settings.enabled = true;
                settings.status_bar = vec![MonitorMetric::Memory];
                settings.interval_secs = 5;
            })
            .detach();
    });
    cx.run_until_parked();
    assert!(host.reading(1));
    assert!(host.script().contains("@@meminfo"));
    assert!(!host.script().contains("@@stat"));
    let weak = monitor.downgrade();
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(workspace);
    drop(monitor);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
    let (_, new_workspace, new_monitor) = cx.update(|cx| {
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Workspace::new(
                        nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx)),
                        window,
                        cx,
                    )
                })
            })
            .unwrap();
        let monitor = cx.new(|cx| HostMonitor::new(&workspace, None, &installed, cx));
        (window, workspace, monitor)
    });
    cx.run_until_parked();
    assert!(host.reading(2));
    assert_ne!(status(cx, &new_monitor), Status::Off);
    drop(new_workspace);
}

#[gpui_kit::test]
fn monitor_page_native_switch_keeps_original_policy_after_unrelated_installation(
    cx: &mut TestAppContext,
) {
    let (installed, other, handle, page) = cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(
            DesignTokens::builtin(),
            (SettingsConfig::in_memory(Default::default())).install(cx),
            cx,
        );
        let installed = crate::install(None, &ready, cx);
        let original = installed.settings().clone();
        let (handle, page) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| crate::MonitorPage::new(original, window, cx))
        })
        .unwrap();
        let other = another_settings(cx);
        (installed, other, handle, page)
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("monitor-enabled", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!installed.settings().setting::<MonitorSettings>(cx).enabled);
        assert!(other.setting::<MonitorSettings>(cx).enabled);
    });
    let weak = page.downgrade();
    cx.update_window(handle, |_, window, _| window.remove_window())
        .unwrap();
    drop(page);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
}

fn number_page(
    store: SettingsConfig,
    cx: &mut TestAppContext,
) -> (
    crate::Installed,
    gpui_kit::AnyWindowHandle,
    Entity<crate::MonitorPage>,
) {
    let result = cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(DesignTokens::builtin(), (store).install(cx), cx);
        let installed = crate::install(None, &ready, cx);
        let original = installed.settings().clone();
        let (window, page) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| crate::MonitorPage::new(original, window, cx))
        })
        .unwrap();
        (installed, window, page)
    });
    cx.update_window(result.1, |_, window, _| window.activate_window())
        .unwrap();
    cx.simulate_window_resize(
        result.1,
        gpui_kit::size(gpui_kit::px(1200.), gpui_kit::px(1600.)),
    );
    cx.run_until_parked();
    result
}

fn type_interval(window: gpui_kit::AnyWindowHandle, text: &str, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click(("monitor-number", 0usize), cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input(text, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn interval_literal(window: gpui_kit::AnyWindowHandle, expected: &str, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let scope: gpui_kit::ElementId = ("monitor-number", 0usize).into();
        assert!(
            gpui_kit::base::test_support::snapshots(window)
                .iter()
                .any(|e| { e.path().contains(&scope) && e.value() == Some(expected) }),
            "actual native interval literal must be {expected}"
        );
    })
    .unwrap();
}

fn assert_interval(
    settings: &nocterm_store::SettingsHandle,
    expected: u32,
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<MonitorSettings>(cx).interval_secs,
            expected
        )
    });
}

#[gpui_kit::test]
fn native_number_validation_debounce_enter_and_blur_keep_the_original_policy(
    cx: &mut TestAppContext,
) {
    let (installed, window, _page) = number_page(SettingsConfig::in_memory(Default::default()), cx);
    let original = cx.update(|cx| {
        installed
            .settings()
            .setting::<MonitorSettings>(cx)
            .interval_secs
    });
    let other = cx.update(another_settings);
    type_interval(window, "0", cx);
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    interval_literal(window, "0", cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(("monitor-number-error", 0usize)).label(),
            Some("Enter a whole number from 1 to 3600.")
        );
    })
    .unwrap();
    assert_interval(installed.settings(), original, cx);
    type_interval(window, "17", cx);
    assert_interval(installed.settings(), original, cx);
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    assert_interval(installed.settings(), 17, cx);
    type_interval(window, "18", cx);
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_interval(installed.settings(), 18, cx);
    type_interval(window, "19", cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("monitor-enabled", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            installed
                .settings()
                .setting::<MonitorSettings>(cx)
                .interval_secs,
            19
        );
        assert_eq!(other.setting::<MonitorSettings>(cx).interval_secs, original);
    });
}

#[gpui_kit::test]
async fn physical_number_publication_refusal_keeps_old_policy_and_current_native_literal(
    cx: &mut TestAppContext,
) {
    let blocked = tempfile::tempdir().unwrap();
    let store = SettingsConfig::new(
        Default::default(),
        nocterm_settings::SettingsFile::new(blocked.path()),
    );
    let (installed, window, _page) = number_page(store, cx);
    let old = cx.update(|cx| {
        installed
            .settings()
            .setting::<MonitorSettings>(cx)
            .interval_secs
    });
    type_interval(window, "44", cx);
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    // A no-op receipt on the same ordered owner settles after the detached number write.
    let barrier = cx.update(|cx| installed.settings().update::<MonitorSettings>(cx, |_| {}));
    cx.run_until_parked();
    barrier.await.unwrap();
    assert!(blocked.path().is_dir());
    cx.update(|cx| {
        assert_eq!(
            installed
                .settings()
                .setting::<MonitorSettings>(cx)
                .interval_secs,
            old
        )
    });
    interval_literal(window, "44", cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find(("monitor-number-error", 0usize)).is_none(),
            "existing detached save has no receipt diagnostic"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn installing_monitor_and_materializing_its_page_starts_no_sampling(cx: &mut TestAppContext) {
    let host = FakeHost::new("Linux");
    let installed = cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(
            DesignTokens::builtin(),
            (SettingsConfig::in_memory(Default::default())).install(cx),
            cx,
        );
        crate::install(Some(host.clone()), &ready, cx)
    });
    cx.run_until_parked();
    assert!(host.programs().is_empty());
    let (_window, _workspace, _page) = cx.update(|cx| {
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Workspace::new(
                        nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx)),
                        window,
                        cx,
                    )
                })
            })
            .unwrap();
        let page = window
            .update(cx, |_, window, cx| {
                let feedback = nocterm_ui::SettingsPageFeedback::new(cx);
                let context =
                    nocterm_ui::SettingsPageContext::new(installed.settings().clone(), feedback);
                crate::settings_page(&installed).create(context, window, cx)
            })
            .unwrap();
        (window, workspace, page)
    });
    cx.run_until_parked();
    assert!(host.programs().is_empty());
}

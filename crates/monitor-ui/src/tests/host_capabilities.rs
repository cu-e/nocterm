//! The monitor's reading reaches a plugin only while the status bar widget shows it.
use super::*;
use crate::{MonitorUsage, register_observed};
use nocterm_plugin_api::{
    HostError,
    capability::monitor::{Usage, UsageNow},
};
use nocterm_plugin_enablement::PluginEnablement;
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::Observe;

struct Rig {
    settings: nocterm_store::SettingsHandle,
    window: gpui_kit::AnyWindowHandle,
    workspace: Entity<Workspace>,
    installed: crate::Installed,
    usage: MonitorUsage,
}

fn rig(cx: &mut TestAppContext) -> Rig {
    cx.update(|cx| {
        gpui_kit::init(cx);
        let mut document = SettingsDocument::default();
        document.register::<PluginEnablement>();
        let settings = SettingsConfig::in_memory(document).install(cx);
        let ready = nocterm_ui::init(DesignTokens::builtin(), settings.clone(), cx);
        let installed = crate::install(None, &ready, cx);
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let deps =
                        nocterm_workspace::test_support::deps_for(&[&crate::plugin::PLUGIN], cx)
                            .with_settings(settings.clone());
                    Workspace::new(deps, window, cx)
                })
            })
            .unwrap();
        Rig {
            settings,
            window,
            workspace,
            installed,
            usage: MonitorUsage::new(),
        }
    })
}

fn register(rig: &Rig, cx: &mut TestAppContext) {
    cx.update_window(rig.window, |_, window, cx| {
        rig.workspace.update(cx, |workspace, cx| {
            register_observed(&rig.installed, &rig.usage, workspace, window, cx).unwrap();
        });
    })
    .unwrap();
    cx.run_until_parked();
}

fn sample(rig: &Rig, cx: &mut TestAppContext) -> Result<Usage, HostError> {
    let plugin = PluginId::user("acme", "meter").unwrap();
    cx.update(|cx| Observe::<UsageNow>::sample(&rig.usage, &plugin, cx))
}

fn frame(host: &FakeHost, busy: u64, idle: u64, cx: &mut TestAppContext) {
    host.print(&format!(
        "@@stat\ncpu  {busy} 0 0 {idle} 0 0 0 0 0 0\n@@meminfo\nMemTotal: 1000\nMemAvailable: 250\n@@end\n"
    ));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn nothing_is_reported_before_the_widget_exists(cx: &mut TestAppContext) {
    let rig = rig(cx);
    assert_eq!(sample(&rig, cx), Err(HostError::Unavailable));
}

#[gpui_kit::test]
fn a_running_monitor_reports_processor_and_memory_use(cx: &mut TestAppContext) {
    let rig = rig(cx);
    register(&rig, cx);
    let host = FakeHost::new("Linux");
    open_session(cx, rig.window, &rig.workspace, remote(&host, true));
    assert_eq!(
        sample(&rig, cx),
        Err(HostError::Unavailable),
        "no reading yet"
    );

    frame(&host, 100, 100, cx);
    assert_eq!(
        sample(&rig, cx),
        Err(HostError::Unavailable),
        "a load needs a second reading"
    );
    frame(&host, 150, 150, cx);
    assert_eq!(
        sample(&rig, cx),
        Ok(Usage {
            cpu_percent: 50.0,
            memory_percent: 75.0
        })
    );
}

#[gpui_kit::test]
fn a_disconnected_host_reports_nothing(cx: &mut TestAppContext) {
    let rig = rig(cx);
    register(&rig, cx);
    let host = FakeHost::new("Linux");
    let item = open_session(cx, rig.window, &rig.workspace, remote(&host, true));
    frame(&host, 100, 100, cx);
    frame(&host, 150, 150, cx);
    assert!(sample(&rig, cx).is_ok());

    item.update(cx, |item, cx| {
        item.session.connected = false;
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    assert_eq!(sample(&rig, cx), Err(HostError::Unavailable));
}

#[gpui_kit::test]
fn a_monitor_that_is_switched_off_reports_nothing(cx: &mut TestAppContext) {
    let rig = rig(cx);
    register(&rig, cx);
    let host = FakeHost::new("Linux");
    open_session(cx, rig.window, &rig.workspace, remote(&host, true));
    frame(&host, 100, 100, cx);
    frame(&host, 150, 150, cx);
    assert!(sample(&rig, cx).is_ok());

    cx.update(|cx| {
        rig.installed
            .settings()
            .update::<crate::MonitorSettings>(cx, |settings| settings.enabled = false)
            .detach()
    });
    cx.run_until_parked();
    assert_eq!(sample(&rig, cx), Err(HostError::Unavailable));
}

#[gpui_kit::test]
fn sampling_never_starts_a_program(cx: &mut TestAppContext) {
    let rig = rig(cx);
    let host = FakeHost::new("Linux");
    open_session(cx, rig.window, &rig.workspace, remote(&host, true));
    for _ in 0..3 {
        assert_eq!(sample(&rig, cx), Err(HostError::Unavailable));
    }
    cx.run_until_parked();
    assert!(
        host.programs().is_empty(),
        "without the widget the provider cannot start a watch"
    );
}

#[gpui_kit::test]
fn a_disabled_monitor_plugin_reports_nothing(cx: &mut TestAppContext) {
    let rig = rig(cx);
    register(&rig, cx);
    let host = FakeHost::new("Linux");
    open_session(cx, rig.window, &rig.workspace, remote(&host, true));
    frame(&host, 100, 100, cx);
    frame(&host, 150, 150, cx);
    assert!(sample(&rig, cx).is_ok());

    let disabled = PluginEnablement {
        disabled: vec![PluginId::new("monitor")],
    };
    let task = cx.update(|cx| {
        rig.settings
            .update::<PluginEnablement>(cx, move |value| *value = disabled)
    });
    cx.run_until_parked();
    cx.foreground_executor().block_test(task).unwrap();
    cx.run_until_parked();
    assert_eq!(
        sample(&rig, cx),
        Err(HostError::Unavailable),
        "the widget is gone, and so is the monitor it owned"
    );
}

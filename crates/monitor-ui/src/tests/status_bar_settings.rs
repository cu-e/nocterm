//! The status bar metrics as a plugin settings field stay in step with the Monitor page.
use super::*;
use nocterm_plugin_model::{ChoiceOption, FieldEdit, FieldKind, SettingValue, toggle_choice};
use nocterm_settings::SettingsSection as _;

fn metrics() -> (Vec<ChoiceOption>, Vec<String>, bool, u8) {
    let field = crate::plugin::PLUGIN
        .settings
        .field("status_bar")
        .expect("the monitor declares status_bar");
    match &field.kind {
        FieldKind::MultiChoice {
            options,
            default,
            ordered,
            max,
        } => (
            options.to_vec(),
            default.iter().map(ToString::to_string).collect(),
            *ordered,
            *max,
        ),
        other => panic!("status_bar is {other:?}"),
    }
}

fn serde_name(metric: MonitorMetric) -> String {
    serde_json::to_value(metric)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn the_plugin_stores_its_settings_in_the_monitor_section() {
    assert_eq!(
        crate::plugin::PLUGIN.settings_section,
        Some(MonitorSettings::KEY)
    );
}

#[test]
fn options_and_labels_are_the_monitor_metrics_in_order() {
    let (options, _, _, max) = metrics();
    assert_eq!(options.len(), MonitorMetric::ALL.len());
    assert_eq!(usize::from(max), MonitorMetric::ALL.len());
    for (option, metric) in options.iter().zip(MonitorMetric::ALL) {
        assert_eq!(option.key, serde_name(metric));
        assert_eq!(option.label, metric.label());
    }
}

#[test]
fn the_default_is_the_default_monitor_settings() {
    let (_, default, _, _) = metrics();
    let expected: Vec<String> = MonitorSettings::default()
        .status_bar
        .into_iter()
        .map(serde_name)
        .collect();
    assert_eq!(default, expected);
}

#[test]
fn toggling_a_metric_matches_the_page_for_every_metric_and_starting_list() {
    let (options, _, ordered, _) = metrics();
    assert!(ordered);
    let starts: [&[MonitorMetric]; 5] = [
        &[],
        &[MonitorMetric::Cpu, MonitorMetric::Memory],
        &[MonitorMetric::Memory, MonitorMetric::Cpu],
        &[
            MonitorMetric::Network,
            MonitorMetric::Swap,
            MonitorMetric::Load,
        ],
        &MonitorMetric::ALL,
    ];
    for start in starts {
        for metric in MonitorMetric::ALL {
            for on in [true, false] {
                let mut page = start.to_vec();
                crate::page::toggle(&mut page, metric, on);
                let current: Vec<String> = start.iter().copied().map(serde_name).collect();
                let schema = toggle_choice(&options, &current, &serde_name(metric), on, ordered);
                let page: Vec<String> = page.into_iter().map(serde_name).collect();
                assert_eq!(schema, page, "{start:?} {metric:?} {on}");
            }
        }
    }
}

#[gpui_kit::test]
fn writing_through_the_binding_changes_the_status_bar_and_runs_no_program(cx: &mut TestAppContext) {
    let host = FakeHost::new("Linux");
    let installed = cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(
            DesignTokens::builtin(),
            SettingsConfig::in_memory(SettingsDocument::default()).install(cx),
            cx,
        );
        crate::install(Some(host.clone()), &ready, cx)
    });
    cx.run_until_parked();
    let source = crate::plugin_settings(&installed);
    cx.update(|cx| {
        let shown = source.values(cx);
        assert_eq!(
            shown.get("status_bar"),
            Some(&SettingValue::Choices(vec!["cpu".into(), "memory".into()]))
        );
        source
            .write(
                "status_bar",
                FieldEdit::Toggle {
                    option: "swap".into(),
                    on: true,
                },
                cx,
            )
            .detach();
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            installed
                .settings()
                .setting::<MonitorSettings>(cx)
                .status_bar,
            vec![
                MonitorMetric::Cpu,
                MonitorMetric::Memory,
                MonitorMetric::Swap
            ]
        );
    });
    assert!(host.programs().is_empty());
}

#[gpui_kit::test]
fn a_write_from_the_form_and_one_from_the_page_both_land(cx: &mut TestAppContext) {
    let (_window, _workspace, _monitor, installed) = fixture_with_installation(cx, None);
    let source = crate::plugin_settings(&installed);
    cx.update(|cx| {
        source
            .write(
                "status_bar",
                FieldEdit::Toggle {
                    option: "load".into(),
                    on: true,
                },
                cx,
            )
            .detach();
        installed
            .settings()
            .update::<MonitorSettings>(cx, |settings| settings.interval_secs = 7)
            .detach();
        source
            .write(
                "status_bar",
                FieldEdit::Toggle {
                    option: "cpu".into(),
                    on: false,
                },
                cx,
            )
            .detach();
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let monitor = installed.settings().setting::<MonitorSettings>(cx);
        assert_eq!(monitor.interval_secs, 7);
        assert_eq!(
            monitor.status_bar,
            vec![MonitorMetric::Memory, MonitorMetric::Load]
        );
    });
}

#[gpui_kit::test]
fn an_unknown_option_is_refused_and_changes_nothing(cx: &mut TestAppContext) {
    let (_window, _workspace, _monitor, installed) = fixture_with_installation(cx, None);
    let source = crate::plugin_settings(&installed);
    let task = cx.update(|cx| {
        source.write(
            "status_bar",
            FieldEdit::Toggle {
                option: "gpu".into(),
                on: true,
            },
            cx,
        )
    });
    cx.run_until_parked();
    let refused = block_on(task).unwrap_err();
    assert_eq!(refused.to_string(), "`gpu` is not one of the options.");
    cx.update(|cx| {
        assert_eq!(
            installed
                .settings()
                .setting::<MonitorSettings>(cx)
                .status_bar,
            MonitorSettings::default().status_bar
        );
    });
}

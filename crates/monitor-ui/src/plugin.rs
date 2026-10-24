//! The host monitor as a system plugin: one status bar widget.
use std::borrow::Cow;

use nocterm_plugin_model::{
    AutoPlace, ChoiceOption, ContributionId, ContributionKind, FieldGroup, FieldKind, FieldSpec,
    Icon, Placement, PluginId, SettingsSchema, SystemContribution, SystemPlugin, Zone,
};

/// The resource summary of the active environment in the status bar.
pub const SUMMARY: SystemContribution = SystemContribution {
    id: ContributionId::new("monitor.summary"),
    kind: ContributionKind::Widget,
    title: "Monitor",
    icon: Icon::Activity,
    placement: Placement {
        zone: Zone::Left,
        order: 60,
        group: None,
        auto: AutoPlace::Show,
    },
    panel: None,
    preview: None,
    pinned: false,
};

/// Monitor: CPU and memory of the active environment.
pub static PLUGIN: SystemPlugin = SystemPlugin {
    id: PluginId::new("monitor"),
    name: "Monitor",
    description: "CPU and memory of the active environment in the status bar",
    required: false,
    contributions: &[SUMMARY],
    settings_section: Some("monitor"),
    settings: SettingsSchema {
        groups: Cow::Borrowed(&[STATUS_BAR_GROUP]),
    },
    settings_page: None,
};

/// The key of the status bar metrics field; it is the serde key in `[monitor]`.
pub(crate) const STATUS_BAR_KEY: &str = "status_bar";

const fn metric(key: &'static str, label: &'static str) -> ChoiceOption {
    ChoiceOption {
        key: Cow::Borrowed(key),
        label: Cow::Borrowed(label),
        help: None,
    }
}

/// One option per `MonitorMetric`, in the order of `MonitorMetric::ALL`.
static METRICS: [ChoiceOption; 10] = [
    metric("cpu", "CPU"),
    metric("cores", "CPU cores"),
    metric("memory", "Memory"),
    metric("swap", "Swap"),
    metric("load", "Load average"),
    metric("uptime", "Uptime"),
    metric("temperature", "Temperature"),
    metric("disks", "Disk space"),
    metric("network", "Network"),
    metric("disk_io", "Disk I/O"),
];

static DEFAULT_METRICS: [Cow<'static, str>; 2] = [Cow::Borrowed("cpu"), Cow::Borrowed("memory")];

static STATUS_BAR_FIELDS: [FieldSpec; 1] = [FieldSpec {
    key: Cow::Borrowed(STATUS_BAR_KEY),
    label: Cow::Borrowed("Metrics in the status bar"),
    help: Some(Cow::Borrowed(
        "What the summary shows. Details open on click and show everything the host reports.",
    )),
    kind: FieldKind::MultiChoice {
        options: Cow::Borrowed(&METRICS),
        default: Cow::Borrowed(&DEFAULT_METRICS),
        ordered: true,
        max: 10,
    },
}];

const STATUS_BAR_GROUP: FieldGroup = FieldGroup {
    title: Cow::Borrowed("Status bar"),
    contribution: Some(ContributionId::new("monitor.summary")),
    fields: Cow::Borrowed(&STATUS_BAR_FIELDS),
};

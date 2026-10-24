//! The clock as a system plugin: one status bar widget, off the Default arrangement.
use std::borrow::Cow;

use nocterm_plugin_model::{
    AutoPlace, ChoiceOption, ContributionId, ContributionKind, FieldGroup, FieldKind, FieldSpec,
    Icon, Placement, PluginId, SettingsSchema, SystemContribution, SystemPlugin, Zone,
};

/// The local time in the status bar. The Default arrangement leaves it in the tray so that
/// adding the plugin changes no window.
pub const TIME: SystemContribution = SystemContribution {
    id: ContributionId::new("clock.time"),
    kind: ContributionKind::Widget,
    title: "Clock",
    icon: Icon::Clock,
    placement: Placement {
        zone: Zone::Right,
        order: 25,
        group: None,
        auto: AutoPlace::Tray,
    },
    panel: None,
    preview: None,
    pinned: false,
};

/// Clock: the local time in the status bar, `HH:MM` unless its settings say otherwise.
pub static PLUGIN: SystemPlugin = SystemPlugin {
    id: PluginId::new("clock"),
    name: "Clock",
    description: "The local time in the status bar",
    required: false,
    contributions: &[TIME],
    settings_section: Some("clock"),
    settings: SettingsSchema {
        version: 1,
        groups: Cow::Borrowed(&[DISPLAY_GROUP]),
    },
    settings_page: None,
};

/// The keys of the fields; they are the serde keys of `[clock]`.
pub(super) const FORMAT_KEY: &str = "format";
pub(super) const SECONDS_KEY: &str = "seconds";
pub(super) const DATE_KEY: &str = "date";

/// One option per `ClockFormat`, in declaration order.
static FORMATS: [ChoiceOption; 2] = [
    ChoiceOption {
        key: Cow::Borrowed("hour24"),
        label: Cow::Borrowed("24-hour"),
        help: None,
    },
    ChoiceOption {
        key: Cow::Borrowed("hour12"),
        label: Cow::Borrowed("12-hour"),
        help: Some(Cow::Borrowed("With AM or PM")),
    },
];

static DISPLAY_FIELDS: [FieldSpec; 3] = [
    FieldSpec {
        renamed_from: None,
        key: Cow::Borrowed(FORMAT_KEY),
        label: Cow::Borrowed("Time format"),
        help: None,
        kind: FieldKind::Choice {
            options: Cow::Borrowed(&FORMATS),
            default: Cow::Borrowed("hour24"),
        },
    },
    FieldSpec {
        renamed_from: None,
        key: Cow::Borrowed(SECONDS_KEY),
        label: Cow::Borrowed("Show seconds"),
        help: Some(Cow::Borrowed(
            "The clock then updates every second instead of every minute.",
        )),
        kind: FieldKind::Toggle { default: false },
    },
    FieldSpec {
        renamed_from: None,
        key: Cow::Borrowed(DATE_KEY),
        label: Cow::Borrowed("Show the date"),
        help: Some(Cow::Borrowed(
            "A short date, such as Oct 24, before the time.",
        )),
        kind: FieldKind::Toggle { default: false },
    },
];

const DISPLAY_GROUP: FieldGroup = FieldGroup {
    title: Cow::Borrowed("Display"),
    contribution: Some(ContributionId::new("clock.time")),
    fields: Cow::Borrowed(&DISPLAY_FIELDS),
};

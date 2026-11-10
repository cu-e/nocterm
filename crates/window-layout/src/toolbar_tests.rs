//! Toolbars along the edges of the tab area: hints, the tray, disabled plugins and edits.

use std::collections::BTreeSet;

use nocterm_plugin_model::{
    AutoPlace, ContributionKind, Edge, PluginCatalog, PluginId, PluginSet, SettingsSchema,
    SystemContribution, SystemPlugin, Zone,
};

use crate::{
    tests::{NONE, SHELL, contribution, groups, id},
    *,
};

const fn toolbar(id: &'static str, edge: Edge, order: u16, auto: AutoPlace) -> SystemContribution {
    let mut toolbar = contribution(id, ContributionKind::Toolbar, Zone::Left, order, None);
    toolbar.edge = Some(edge);
    toolbar.placement.auto = auto;
    toolbar
}

static BARS: SystemPlugin = SystemPlugin {
    id: PluginId::new("bars"),
    name: "Bars",
    description: "Toolbars that can be switched off",
    required: false,
    contributions: &[
        toolbar("bars.controls", Edge::Top, 10, AutoPlace::Show),
        toolbar("bars.second", Edge::Top, 20, AutoPlace::Show),
        toolbar("bars.side", Edge::Left, 10, AutoPlace::Tray),
    ],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: None,
};

fn catalog() -> PluginSet {
    PluginCatalog::new(&groups(), &[SHELL], &[&BARS])
        .expect("a valid catalog")
        .into()
}

fn names(ids: &[nocterm_plugin_model::ContributionId]) -> Vec<&str> {
    ids.iter().map(|id| id.as_str()).collect()
}

fn resolved(preset: &LayoutPreset, disabled: &BTreeSet<PluginId>) -> ResolvedLayout {
    resolve(preset, &catalog(), disabled)
}

#[test]
fn a_toolbar_that_asks_to_be_shown_runs_along_its_hinted_edge_in_order() {
    let layout = resolved(&LayoutPreset::default(), &NONE);
    assert_eq!(
        names(layout.toolbars(Edge::Top)),
        ["bars.controls", "bars.second"]
    );
    assert!(layout.toolbars(Edge::Left).is_empty(), "a tray hint waits");
    assert_eq!(layout.toolbar_edge(&id("bars.controls")), Some(Edge::Top));
    assert!(layout.is_placed(&id("bars.controls")));
    assert!(!layout.is_placed(&id("bars.side")));
}

#[test]
fn toolbars_never_enter_the_status_bar() {
    let layout = resolved(&LayoutPreset::default(), &NONE);
    for zone in [Zone::Left, Zone::Center, Zone::Right] {
        let shown = crate::tests::show(layout.zone(zone));
        assert!(
            shown.iter().all(|entry| !entry.contains("bars.")),
            "{zone:?}: {shown:?}"
        );
    }
}

#[test]
fn a_preset_moves_a_toolbar_to_another_edge_and_the_tray_hides_one() {
    let preset = LayoutPreset {
        toolbars: Toolbars {
            bottom: vec![id("bars.controls"), id("bars.side")],
            ..Toolbars::default()
        },
        tray: vec![id("bars.second")],
        ..LayoutPreset::default()
    };
    let layout = resolved(&preset, &NONE);
    assert_eq!(
        names(layout.toolbars(Edge::Bottom)),
        ["bars.controls", "bars.side"]
    );
    assert!(layout.toolbars(Edge::Top).is_empty());
}

#[test]
fn an_edge_shows_only_toolbars_and_each_once() {
    let preset = LayoutPreset {
        toolbars: Toolbars {
            top: vec![id("shell.notices"), id("bars.controls"), id("missing.bar")],
            right: vec![id("bars.controls")],
            ..Toolbars::default()
        },
        ..LayoutPreset::default()
    };
    let layout = resolved(&preset, &NONE);
    assert_eq!(
        names(layout.toolbars(Edge::Top)),
        ["bars.controls", "bars.second"]
    );
    assert!(layout.toolbars(Edge::Right).is_empty());
}

#[test]
fn the_toolbars_of_a_disabled_plugin_are_not_shown_and_stay_in_the_preset() {
    let disabled = BTreeSet::from([PluginId::new("bars")]);
    let preset = LayoutPreset {
        toolbars: Toolbars {
            right: vec![id("bars.side")],
            ..Toolbars::default()
        },
        ..LayoutPreset::default()
    };
    let layout = resolved(&preset, &disabled);
    for edge in Edge::ALL {
        assert!(layout.toolbars(edge).is_empty(), "{edge:?}");
    }
    assert_eq!(names(layout.disabled()), ["bars.side"]);
}

fn edit_selected(settings: &mut LayoutSettings, op: EditOp) -> Result<EditOutcome, EditError> {
    edit(settings, false, &catalog(), EditTarget::Selected, op)
}

fn shown(settings: &LayoutSettings, edge: Edge) -> Vec<String> {
    let active = settings.active.clone().unwrap_or(BuiltIn::Default.id());
    let layout = settings.resolve(&active, &catalog(), &NONE);
    names(layout.toolbars(edge))
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[test]
fn showing_a_toolbar_on_the_default_layout_makes_a_custom_layout_that_shows_it() {
    let mut settings = LayoutSettings::default();
    edit_selected(
        &mut settings,
        EditOp::ShowToolbar {
            id: id("bars.side"),
            edge: Edge::Right,
        },
    )
    .expect("a toolbar can be shown");
    assert_eq!(settings.presets.len(), 1, "the default was copied");
    assert_eq!(shown(&settings, Edge::Right), ["bars.side"]);
    assert_eq!(
        shown(&settings, Edge::Top),
        ["bars.controls", "bars.second"],
        "the hinted toolbars were written out"
    );

    edit_selected(
        &mut settings,
        EditOp::ShowToolbar {
            id: id("bars.controls"),
            edge: Edge::Right,
        },
    )
    .expect("a shown toolbar can move");
    assert_eq!(
        shown(&settings, Edge::Right),
        ["bars.side", "bars.controls"]
    );
    assert_eq!(shown(&settings, Edge::Top), ["bars.second"]);
}

#[test]
fn hiding_a_toolbar_puts_it_in_the_tray_and_restoring_it_follows_its_hint() {
    let mut settings = LayoutSettings::default();
    edit_selected(
        &mut settings,
        EditOp::RemoveToTray {
            id: id("bars.controls"),
        },
    )
    .expect("a toolbar can be hidden");
    assert_eq!(shown(&settings, Edge::Top), ["bars.second"]);
    let stored = &settings.presets[0];
    assert!(stored.tray.contains(&id("bars.controls")));
    assert_eq!(stored.toolbars.edge_of(&id("bars.controls")), None);

    edit_selected(
        &mut settings,
        EditOp::Restore {
            id: id("bars.controls"),
        },
    )
    .expect("a toolbar can be restored");
    assert_eq!(
        shown(&settings, Edge::Top),
        ["bars.second", "bars.controls"]
    );
    assert!(!settings.presets[0].tray.contains(&id("bars.controls")));
}

#[test]
fn only_a_toolbar_can_be_shown_along_an_edge_and_a_toolbar_never_enters_a_zone() {
    let mut settings = LayoutSettings::default();
    let widget = edit_selected(
        &mut settings,
        EditOp::ShowToolbar {
            id: id("shell.notices"),
            edge: Edge::Top,
        },
    );
    assert!(matches!(widget, Err(EditError::WrongKind(_))), "{widget:?}");
    let toolbar = edit_selected(
        &mut settings,
        EditOp::ToZone {
            subject: Subject::Contribution(id("bars.controls")),
            zone: Zone::Left,
        },
    );
    assert!(
        matches!(toolbar, Err(EditError::WrongKind(_))),
        "{toolbar:?}"
    );
}

#[test]
fn a_preset_without_toolbars_is_written_without_the_key_and_one_with_them_round_trips() {
    let empty = toml::to_string(&LayoutPreset::default()).expect("serializes");
    assert!(!empty.contains("toolbars"), "{empty}");
    let preset = LayoutPreset {
        toolbars: Toolbars {
            left: vec![id("bars.side")],
            ..Toolbars::default()
        },
        ..LayoutPreset::default()
    };
    let text = toml::to_string(&preset).expect("serializes");
    let back: LayoutPreset = toml::from_str(&text).expect("reads back");
    assert_eq!(back, preset);
}

#[test]
fn moving_a_toolbar_in_a_stored_layout_is_an_edit_the_window_applies() {
    let mut settings = LayoutSettings::default();
    edit_selected(
        &mut settings,
        EditOp::ShowToolbar {
            id: id("bars.side"),
            edge: Edge::Right,
        },
    )
    .expect("a toolbar can be shown");
    let before = settings.clone();
    edit_selected(
        &mut settings,
        EditOp::ShowToolbar {
            id: id("bars.side"),
            edge: Edge::Left,
        },
    )
    .expect("a shown toolbar can move");
    assert_eq!(
        transition(&before, &settings, false),
        LayoutTransition::Edit
    );
}

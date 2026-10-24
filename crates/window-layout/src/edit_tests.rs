//! Edits of the presets: copy-on-write, targets, naming, deleting and undo.

use std::collections::BTreeSet;

use nocterm_plugin_model::{PluginCatalog, PluginId, SettingsSchema, SystemPlugin, Zone};

use crate::{
    tests::{NONE, SHELL, groups, id, panel, show, widget},
    *,
};

static TOOLS: SystemPlugin = SystemPlugin {
    id: PluginId::new("tools"),
    name: "Tools",
    description: "Switch-off-able items",
    required: false,
    contributions: &[
        widget("tools.clock", Zone::Right, 25),
        panel("tools.list", 40),
    ],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: None,
};

pub(crate) fn catalog() -> PluginCatalog {
    PluginCatalog::new(&groups(), &[SHELL], &[&TOOLS]).expect("a valid catalog")
}

pub(crate) fn pid(id: &str) -> PresetId {
    PresetId::from_owned(id.to_owned())
}

pub(crate) fn item(name: &'static str) -> Subject {
    Subject::Contribution(id(name))
}

pub(crate) fn stored(id: &str, name: &str) -> LayoutPreset {
    LayoutPreset {
        id: pid(id),
        name: name.to_owned(),
        ..LayoutPreset::default()
    }
}

pub(crate) fn settings(active: Option<&str>, presets: Vec<LayoutPreset>) -> LayoutSettings {
    LayoutSettings {
        active: active.map(pid),
        presets,
    }
}

pub(crate) fn apply(settings: &mut LayoutSettings, op: EditOp) -> Result<EditOutcome, EditError> {
    edit(settings, false, &catalog(), EditTarget::Selected, op)
}

pub(crate) fn apply_to(
    settings: &mut LayoutSettings,
    target: &str,
    op: EditOp,
) -> Result<EditOutcome, EditError> {
    let target = EditTarget::Preset(pid(target));
    edit(settings, false, &catalog(), target, op)
}

pub(crate) fn mv(name: &'static str, by: i32) -> EditOp {
    EditOp::Move {
        subject: item(name),
        by,
    }
}

/// A zone of a layout, with every plugin enabled.
pub(crate) fn zone(settings: &LayoutSettings, layout: &str, zone: Zone) -> Vec<String> {
    let resolved = settings.resolve(&pid(layout), &catalog(), &NONE);
    show(resolved.zone(zone))
}

pub(crate) const RIGHT: [&str; 5] = [
    "shell.notices",
    "shell.transfers",
    "tools.clock",
    "shell.terminal",
    "[assistant: shell.agent]",
];

#[test]
fn the_first_edit_of_default_makes_an_active_custom_preset() {
    let mut document = LayoutSettings::default();

    let outcome = apply(&mut document, mv("shell.transfers", -1)).unwrap();

    assert_eq!(document.presets.len(), 1);
    assert_eq!(document.presets[0].name, "Custom");
    assert_eq!(document.active, Some(pid("p1")));
    assert_eq!(outcome.selected, pid("p1"));
    assert!(outcome.before.is_empty());
    assert_eq!(outcome.after, document.presets);
    assert_eq!(
        zone(&document, "p1", Zone::Right),
        [
            "shell.transfers",
            "shell.notices",
            "tools.clock",
            "shell.terminal",
            "[assistant: shell.agent]"
        ]
    );
}

#[test]
fn two_selected_edits_on_default_land_on_one_custom_preset() {
    let mut document = LayoutSettings::default();

    apply(&mut document, mv("shell.transfers", -1)).unwrap();
    apply(&mut document, mv("shell.terminal", -1)).unwrap();

    assert_eq!(document.presets.len(), 1);
    assert_eq!(
        zone(&document, "p1", Zone::Right),
        [
            "shell.transfers",
            "shell.notices",
            "shell.terminal",
            "tools.clock",
            "[assistant: shell.agent]"
        ]
    );
}

#[test]
fn a_written_out_built_in_resolves_exactly_like_the_built_in() {
    for (legacy_swapped, built_in) in [(false, BuiltIn::Default), (true, BuiltIn::Swapped)] {
        let mut document = LayoutSettings::default();

        edit(
            &mut document,
            legacy_swapped,
            &catalog(),
            EditTarget::Selected,
            mv("shell.notices", 0),
        )
        .unwrap();

        let copy = document.resolve(&pid("p1"), &catalog(), &NONE);
        assert_eq!(copy, built_in.resolve(&catalog(), &NONE));
    }
}

#[test]
fn writing_out_keeps_the_entries_of_disabled_plugins_where_their_hints_put_them() {
    let mut document = LayoutSettings::default();

    apply(&mut document, mv("shell.transfers", -1)).unwrap();

    let off = BTreeSet::from([PluginId::new("tools")]);
    let hidden = document.resolve(&pid("p1"), &catalog(), &off);
    assert!(!hidden.is_placed(&id("tools.clock")));
    assert!(!hidden.is_placed(&id("tools.list")));
    assert_eq!(zone(&document, "p1", Zone::Right)[2], "tools.clock");
    assert!(document.presets[0].bar.right.contains(&BarItem::Item {
        id: id("tools.clock")
    }));
}

#[test]
fn an_edit_of_a_stored_preset_leaves_the_selection_and_the_other_presets_alone() {
    let mut document = settings(Some("p1"), vec![stored("p1", "One"), stored("p2", "Two")]);

    let outcome = apply_to(&mut document, "p2", mv("shell.transfers", -1)).unwrap();

    assert_eq!(document.active, Some(pid("p1")));
    assert_eq!(outcome.selected, pid("p2"));
    assert_eq!(document.presets[0], stored("p1", "One"));
    assert_eq!(zone(&document, "p2", Zone::Right)[0], "shell.transfers");
}

#[test]
fn a_built_in_that_is_not_selected_refuses_every_edit_but_duplicate() {
    let mut document = settings(Some("p2"), vec![stored("p2", "Two")]);
    let unchanged = document.clone();

    let refused = apply_to(&mut document, "default", mv("shell.transfers", -1));

    assert_eq!(refused, Err(EditError::BuiltinReadOnly(pid("default"))));
    assert_eq!(document, unchanged);
}

#[test]
fn duplicating_a_built_in_that_is_not_selected_keeps_the_selection() {
    let mut document = settings(Some("p2"), vec![stored("p2", "Two")]);

    let outcome = apply_to(&mut document, "default", EditOp::Duplicate).unwrap();

    assert_eq!(document.active, Some(pid("p2")));
    assert_eq!(outcome.selected, pid("p3"));
    assert_eq!(document.presets.len(), 2);
    assert_eq!(document.presets[1].name, "Default copy");
    assert_eq!(
        document.resolve(&pid("p3"), &catalog(), &NONE),
        BuiltIn::Default.resolve(&catalog(), &NONE)
    );
}

#[test]
fn the_selected_built_in_named_by_id_is_edited_like_selected() {
    let mut document = LayoutSettings::default();

    apply_to(&mut document, "default", mv("shell.transfers", -1)).unwrap();

    assert_eq!(document.presets.len(), 1);
    assert_eq!(document.active, Some(pid("p1")));
}

#[test]
fn a_preset_that_is_gone_is_reported_and_nothing_changes() {
    let mut document = settings(None, vec![stored("p1", "One")]);
    let unchanged = document.clone();

    let refused = apply_to(&mut document, "p9", EditOp::Duplicate);

    assert_eq!(refused, Err(EditError::PresetGone(pid("p9"))));
    assert_eq!(document, unchanged);
}

#[test]
fn rename_checks_the_name_and_a_built_in_keeps_its_own() {
    let mut document = settings(Some("p1"), vec![stored("p1", "One"), stored("p2", "Two")]);

    apply(
        &mut document,
        EditOp::Rename {
            name: "  Focus ".into(),
        },
    )
    .unwrap();
    assert_eq!(document.presets[0].name, "Focus");
    let reserved = apply(
        &mut document,
        EditOp::Rename {
            name: "default".into(),
        },
    );
    assert_eq!(
        reserved,
        Err(EditError::Name(NameError::Reserved("default".into())))
    );
    let taken = apply_to(
        &mut document,
        "p2",
        EditOp::Rename {
            name: "focus".into(),
        },
    );
    assert_eq!(
        taken,
        Err(EditError::Name(NameError::Duplicate("focus".into())))
    );

    let mut plain = LayoutSettings::default();
    let refused = apply(
        &mut plain,
        EditOp::Rename {
            name: "Mine".into(),
        },
    );
    assert_eq!(refused, Err(EditError::BuiltinReadOnly(pid("default"))));
    assert_eq!(plain, LayoutSettings::default());
}

#[test]
fn a_copy_gets_a_free_name_and_does_not_become_selected() {
    let mut document = settings(Some("p1"), vec![stored("p1", "One")]);

    apply(&mut document, EditOp::Duplicate).unwrap();
    let again = apply(&mut document, EditOp::Duplicate).unwrap();

    let names: Vec<_> = document.presets.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["One", "One copy", "One copy 2"]);
    assert_eq!(again.selected, pid("p3"));
    assert_eq!(document.active, Some(pid("p1")));
}

#[test]
fn reset_empties_a_preset_so_the_hints_arrange_it_again() {
    let mut document = LayoutSettings::default();
    apply(&mut document, mv("shell.transfers", -1)).unwrap();

    apply(&mut document, EditOp::Reset).unwrap();

    assert_eq!(document.presets[0].bar, Bar::default());
    assert_eq!(zone(&document, "p1", Zone::Right), RIGHT);
}

#[test]
fn deleting_the_selected_preset_selects_the_default_and_undo_brings_it_back() {
    let mut document = LayoutSettings::default();
    apply(&mut document, mv("shell.transfers", -1)).unwrap();
    let original = document.presets.clone();

    let deleted = apply(&mut document, EditOp::Delete).unwrap();

    assert!(document.presets.is_empty());
    assert_eq!(document.active, None);
    assert_eq!(deleted.selected, pid("default"));

    apply(&mut document, EditOp::RestorePresets(deleted.before)).unwrap();
    assert_eq!(document.presets, original);
    assert_eq!(document.active, None);
    assert_eq!(select(&document, false), pid("default"));
}

#[test]
fn restoring_presets_never_touches_the_selection() {
    let mut document = settings(Some("p2"), vec![stored("p1", "One"), stored("p2", "Two")]);

    let outcome = apply(
        &mut document,
        EditOp::RestorePresets(vec![stored("p1", "One")]),
    )
    .unwrap();

    assert_eq!(document.active, Some(pid("p2")));
    assert_eq!(outcome.selected, pid("default"));
    assert_eq!(outcome.before.len(), 2);
}

#[test]
fn undoing_the_first_edit_returns_to_the_default_and_redo_returns_to_the_copy() {
    let mut document = LayoutSettings::default();
    let first = apply(&mut document, mv("shell.transfers", -1)).unwrap();

    apply(&mut document, EditOp::RestorePresets(first.before.clone())).unwrap();
    assert_eq!(select(&document, false), pid("default"));
    assert_eq!(
        zone(&document, "default", Zone::Right),
        RIGHT,
        "the default shows"
    );

    apply(&mut document, EditOp::RestorePresets(first.after.clone())).unwrap();
    assert_eq!(select(&document, false), pid("p1"));
    assert_eq!(zone(&document, "p1", Zone::Right)[0], "shell.transfers");
}

#[test]
fn the_document_keeps_at_most_the_most_presets() {
    let presets = (1..=MAX_PRESETS)
        .map(|n| stored(&format!("p{n}"), &format!("Layout {n}")))
        .collect();
    let mut document = settings(Some("p1"), presets);
    let unchanged = document.clone();

    let full = apply(&mut document, EditOp::Duplicate);
    let copy = apply_to(&mut document, "default", EditOp::Duplicate);

    assert_eq!(full, Err(EditError::Full(Limit::Presets)));
    assert_eq!(copy, Err(EditError::Full(Limit::Presets)));
    assert_eq!(document, unchanged);
}

#[test]
fn an_edit_that_changes_nothing_keeps_the_preset_as_it_was() {
    let mut document = LayoutSettings::default();
    apply(&mut document, mv("shell.transfers", -1)).unwrap();
    let before = document.clone();

    let outcome = apply(&mut document, mv("shell.notices", 0)).unwrap();

    assert_eq!(document, before);
    assert_eq!(outcome.before, outcome.after);
}

#[test]
fn a_transition_after_the_first_edit_is_an_edit_of_the_new_selection() {
    let before = LayoutSettings::default();
    let mut after = before.clone();

    apply(&mut after, mv("shell.transfers", -1)).unwrap();

    assert_eq!(transition(&before, &after, false), LayoutTransition::Edit);
}

#[test]
fn a_copy_made_after_undoing_the_first_edit_does_not_take_the_id_the_selection_waits_for() {
    let mut document = LayoutSettings::default();
    let first = apply(&mut document, mv("shell.transfers", -1)).unwrap();
    apply(&mut document, EditOp::RestorePresets(first.before)).unwrap();
    assert_eq!(
        document.active,
        Some(pid("p1")),
        "the selection waits for p1"
    );

    let copy = apply_to(&mut document, "swapped", EditOp::Duplicate).unwrap();

    assert_ne!(copy.selected, pid("p1"));
    assert_eq!(copy.selected, pid("p2"));
    assert_eq!(select(&document, false), pid("default"), "the footer stays");
}

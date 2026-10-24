//! The `[layouts]` section: reading, sanitizing, selecting and the transitions between edits.

use nocterm_plugin_model::{ContributionId, GroupId, Side};
use nocterm_settings::{SettingsDocument, SettingsSection};

use crate::*;

fn id(id: &'static str) -> ContributionId {
    ContributionId::new(id)
}

fn pid(id: &str) -> PresetId {
    PresetId::from_owned(id.to_owned())
}

fn stored(id: &str, name: &str) -> LayoutPreset {
    LayoutPreset {
        id: pid(id),
        name: name.to_owned(),
        ..LayoutPreset::default()
    }
}

fn settings(active: Option<&str>, presets: Vec<LayoutPreset>) -> LayoutSettings {
    LayoutSettings {
        active: active.map(pid),
        presets,
    }
}

fn read(text: &str) -> LayoutSettings {
    let mut document = SettingsDocument::from_table(text.parse().unwrap());
    document.register::<LayoutSettings>();
    document.get::<LayoutSettings>().clone()
}

fn sanitized(mut settings: LayoutSettings) -> LayoutSettings {
    settings.sanitize();
    settings
}

const FOCUS: &str = r#"
[layouts]
active = "p1"

[[layouts.presets]]
id = "p1"
name = "Focus"
tray = ["files.transfers"]
open = ["files.explorer", "workspace.local-terminal"]

[layouts.presets.bar]
left = [
    { group = "navigation", primary = true, members = ["connections.servers", { separator = true }, "files.explorer"] },
    { item = "monitor.summary" },
]
center = [{ item = "workspace.notifications" }]
right = [{ separator = true }, { group = "assistant", side = "left", members = ["agent.chats"] }]
"#;

fn focus() -> LayoutPreset {
    LayoutPreset {
        id: pid("p1"),
        name: "Focus".into(),
        bar: Bar {
            left: vec![
                BarItem::Group {
                    id: GroupId::new("navigation"),
                    side: None,
                    primary: true,
                    members: vec![
                        GroupEntry::Panel(id("connections.servers")),
                        GroupEntry::Separator,
                        GroupEntry::Panel(id("files.explorer")),
                    ],
                },
                BarItem::Item {
                    id: id("monitor.summary"),
                },
            ],
            center: vec![BarItem::Item {
                id: id("workspace.notifications"),
            }],
            right: vec![
                BarItem::Separator,
                BarItem::Group {
                    id: GroupId::new("assistant"),
                    side: Some(Side::Left),
                    primary: false,
                    members: vec![GroupEntry::Panel(id("agent.chats"))],
                },
            ],
        },
        tray: vec![id("files.transfers")],
        open: vec![id("files.explorer"), id("workspace.local-terminal")],
    }
}

#[test]
fn the_documented_layouts_table_is_read() {
    assert_eq!(read(FOCUS), settings(Some("p1"), vec![focus()]));
}

#[test]
fn every_bar_item_and_group_entry_survives_a_write_and_a_read() {
    let original = settings(Some("p1"), vec![focus()]);
    let mut document = SettingsDocument::default();
    document.register::<LayoutSettings>();
    document.set(original.clone());
    let text = toml::to_string(&document.to_table()).unwrap();
    assert_eq!(read(&text), original, "{text}");
}

#[test]
fn a_missing_table_reads_as_no_presets_and_nothing_selected() {
    assert_eq!(read(""), LayoutSettings::default());
}

#[test]
fn a_table_that_does_not_read_is_reported_and_a_good_one_is_not() {
    let document = |text: &str| {
        let mut document = SettingsDocument::from_table(text.parse().unwrap());
        document.register::<LayoutSettings>();
        document
    };
    let broken = document(
        "[layouts]\nactive = \"p1\"\n[[layouts.presets]]\nid = \"p1\"\n[layouts.presets.bar]\nleft = [{ separator = false }]\n",
    );
    assert!(LayoutSettings::read_error(&broken).is_some());
    assert_eq!(LayoutSettings::read_error(&document(FOCUS)), None);
    assert_eq!(LayoutSettings::read_error(&document("")), None);
}

#[test]
fn a_separator_must_be_true() {
    let mut document = SettingsDocument::from_table(
        "[layouts]\n[[layouts.presets]]\nid = \"p1\"\n[layouts.presets.bar]\nleft = [{ separator = false }]\n"
            .parse()
            .unwrap(),
    );
    document.register::<LayoutSettings>();
    assert_eq!(
        document.get::<LayoutSettings>(),
        &LayoutSettings::default(),
        "the broken table falls back to defaults on its own"
    );
    assert!(!document.errors().is_empty());
}

#[test]
fn the_schema_types_contribution_group_and_preset_ids_as_strings() {
    let schema = serde_json::to_value(schemars::schema_for!(LayoutSettings)).unwrap();
    let defs = &schema["$defs"];
    let variants = defs["BarItem"]["anyOf"].as_array().unwrap();
    let typed = |key: &str| {
        variants
            .iter()
            .find_map(|variant| variant["properties"][key]["type"].as_str())
    };
    assert_eq!(typed("item"), Some("string"));
    assert_eq!(typed("group"), Some("string"));
    assert_eq!(defs["GroupEntry"]["anyOf"][0]["type"], "string");
    for list in ["tray", "open"] {
        assert_eq!(
            defs["LayoutPreset"]["properties"][list]["items"]["type"],
            "string"
        );
    }
    assert_eq!(defs["PresetId"]["type"], "string");
}

#[test]
fn select_prefers_a_known_active_layout_then_the_legacy_flag_then_default() {
    let with_p1 = settings(Some("p1"), vec![stored("p1", "One")]);
    assert_eq!(select(&with_p1, false), pid("p1"));
    assert_eq!(
        select(&with_p1, true),
        pid("p1"),
        "active beats the legacy flag"
    );
    assert_eq!(
        select(&settings(Some("swapped"), vec![]), false),
        pid("swapped")
    );
    assert_eq!(
        select(&settings(Some("default"), vec![]), true),
        pid("default")
    );
    assert_eq!(select(&settings(None, vec![]), true), pid("swapped"));
    assert_eq!(select(&settings(None, vec![]), false), pid("default"));
    assert_eq!(
        select(&settings(Some("gone"), vec![]), true),
        pid("swapped"),
        "an unknown active falls through to the legacy flag"
    );
    assert_eq!(
        select(&settings(Some("gone"), vec![]), false),
        pid("default")
    );
}

#[test]
fn sanitize_gives_missing_invalid_reserved_and_repeated_ids_a_fresh_number() {
    let clean = sanitized(settings(
        None,
        vec![
            stored("p4", "Four"),
            stored("", "Missing"),
            stored("default", "Reserved"),
            stored("p4", "Repeated"),
            stored("Not Valid", "Invalid"),
            stored("work", "Hand written"),
        ],
    ));
    let ids: Vec<_> = clean.presets.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["p4", "p5", "p6", "p7", "p8", "work"]);
}

#[test]
fn next_id_follows_the_largest_stored_number_and_may_reuse_a_freed_one() {
    let two = settings(None, vec![stored("p1", "A"), stored("p3", "B")]);
    assert_eq!(two.next_id(), pid("p4"));
    let freed = settings(None, vec![stored("p1", "A")]);
    assert_eq!(freed.next_id(), pid("p2"));
    assert_eq!(LayoutSettings::default().next_id(), pid("p1"));
}

#[test]
fn ids_continue_below_the_largest_number_once_it_cannot_grow() {
    let top = format!("p{}", u32::MAX);
    let clean = sanitized(settings(
        None,
        vec![
            stored(&top, "Top"),
            stored("", "Missing"),
            stored("", "Also"),
        ],
    ));
    let ids: Vec<_> = clean.presets.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, [top.as_str(), "p1", "p2"], "no id repeats");
    assert_eq!(clean.next_id(), pid("p3"));
}

#[test]
fn a_selection_follows_the_preset_whose_invalid_id_was_replaced() {
    let clean = sanitized(settings(
        Some("Not Valid"),
        vec![stored("p1", "One"), stored("Not Valid", "Hand written")],
    ));
    assert_eq!(clean.presets[1].id, pid("p2"));
    assert_eq!(clean.active, Some(pid("p2")));
    assert_eq!(
        select(&clean, false),
        pid("p2"),
        "the layout stays selected"
    );
}

#[test]
fn a_selection_of_a_built_in_or_of_a_preset_that_kept_its_id_stays() {
    let reserved = sanitized(settings(
        Some("default"),
        vec![stored("default", "Looks like a built-in")],
    ));
    assert_eq!(reserved.active, Some(pid("default")));
    let repeated = sanitized(settings(
        Some("p1"),
        vec![stored("p1", "First"), stored("p1", "Second")],
    ));
    assert_eq!(repeated.active, Some(pid("p1")));
    assert_eq!(repeated.presets[0].id, pid("p1"));
}

#[test]
fn sanitize_makes_names_short_distinct_and_unlike_the_built_ins() {
    let long = "x".repeat(60);
    let clean = sanitized(settings(
        None,
        vec![
            stored("p1", "Default"),
            stored("p2", "  Swapped Sides "),
            stored("p3", "Work"),
            stored("p4", "work"),
            stored("p5", ""),
            stored("p6", &long),
        ],
    ));
    let names: Vec<_> = clean.presets.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names[..5],
        ["Default 2", "Swapped Sides 2", "Work", "work 2", "Layout"]
    );
    assert_eq!(names[5].chars().count(), MAX_NAME_CHARS);
}

#[test]
fn sanitize_keeps_a_changed_name_stable_on_a_second_pass() {
    let once = sanitized(settings(
        None,
        vec![stored("p1", "Default"), stored("p2", "Default")],
    ));
    let twice = sanitized(once.clone());
    assert_eq!(once, twice);
}

#[test]
fn sanitize_limits_presets_zones_and_groups() {
    let mut preset = stored("p1", "Big");
    preset.bar.left = (0..80).map(|_| BarItem::Separator).collect();
    preset.bar.right = vec![BarItem::Group {
        id: GroupId::new("navigation"),
        side: None,
        primary: false,
        members: (0..40).map(|_| GroupEntry::Separator).collect(),
    }];
    let many = (1..=40).map(|n| stored(&format!("p{n}"), &format!("L{n}")));
    let clean = sanitized(settings(
        None,
        std::iter::once(preset).chain(many).collect(),
    ));
    assert_eq!(clean.presets.len(), MAX_PRESETS);
    assert_eq!(clean.presets[0].bar.left.len(), MAX_ZONE_ITEMS);
    let BarItem::Group { members, .. } = &clean.presets[0].bar.right[0] else {
        panic!("a group");
    };
    assert_eq!(members.len(), MAX_GROUP_MEMBERS);
}

#[test]
fn sanitize_never_touches_the_selection() {
    let clean = sanitized(settings(Some("gone"), vec![stored("p1", "One")]));
    assert_eq!(clean.active, Some(pid("gone")));
}

#[test]
fn validate_name_trims_and_refuses_empty_long_reserved_and_taken_names() {
    let existing = settings(None, vec![stored("p1", "Focus"), stored("p2", "Wide")]);
    assert_eq!(
        validate_name(&existing, None, "  Review "),
        Ok("Review".into())
    );
    assert_eq!(validate_name(&existing, None, "   "), Err(NameError::Empty));
    assert_eq!(
        validate_name(&existing, None, &"y".repeat(MAX_NAME_CHARS + 1)),
        Err(NameError::TooLong)
    );
    assert!(validate_name(&existing, None, &"y".repeat(MAX_NAME_CHARS)).is_ok());
    assert_eq!(
        validate_name(&existing, None, "swapped sides"),
        Err(NameError::Reserved("swapped sides".into()))
    );
    assert_eq!(
        validate_name(&existing, None, "FOCUS"),
        Err(NameError::Duplicate("FOCUS".into()))
    );
    assert_eq!(
        validate_name(&existing, Some(&pid("p1")), "focus"),
        Ok("focus".into()),
        "a preset may keep its own name"
    );
}

mod transitions {
    use super::*;

    fn with_open(mut preset: LayoutPreset, open: &'static str) -> LayoutPreset {
        preset.open = vec![id(open)];
        preset
    }

    #[test]
    fn choosing_another_stored_preset_or_a_built_in_switches() {
        let prev = settings(Some("p1"), vec![stored("p1", "A"), stored("p2", "B")]);
        let to_stored = LayoutSettings {
            active: Some(pid("p2")),
            ..prev.clone()
        };
        assert_eq!(
            transition(&prev, &to_stored, false),
            LayoutTransition::Switch
        );
        let to_built_in = LayoutSettings {
            active: Some(pid("swapped")),
            ..prev.clone()
        };
        assert_eq!(
            transition(&prev, &to_built_in, false),
            LayoutTransition::Switch
        );
        assert_eq!(
            transition(&to_built_in, &prev, false),
            LayoutTransition::Switch
        );
    }

    #[test]
    fn the_first_edit_of_a_built_in_is_an_edit() {
        let prev = settings(None, vec![]);
        let next = settings(Some("p1"), vec![stored("p1", "Custom")]);
        assert_eq!(transition(&prev, &next, false), LayoutTransition::Edit);
        assert_eq!(
            transition(&settings(Some("swapped"), vec![]), &next, false),
            LayoutTransition::Edit
        );
    }

    #[test]
    fn duplicating_and_activating_a_preset_is_an_edit() {
        let prev = settings(Some("p1"), vec![stored("p1", "A")]);
        let next = settings(Some("p2"), vec![stored("p1", "A"), stored("p2", "A copy")]);
        assert_eq!(transition(&prev, &next, false), LayoutTransition::Edit);
    }

    #[test]
    fn changing_the_selected_presets_contents_is_an_edit() {
        let prev = settings(Some("p1"), vec![stored("p1", "A")]);
        let next = settings(
            Some("p1"),
            vec![with_open(stored("p1", "A"), "files.explorer")],
        );
        assert_eq!(transition(&prev, &next, false), LayoutTransition::Edit);
    }

    #[test]
    fn renaming_the_selected_preset_changes_nothing_a_window_shows() {
        let prev = settings(Some("p1"), vec![stored("p1", "A")]);
        let next = settings(Some("p1"), vec![stored("p1", "B")]);
        assert_eq!(transition(&prev, &next, false), LayoutTransition::None);
    }

    #[test]
    fn editing_a_preset_that_is_not_selected_changes_nothing_a_window_shows() {
        let prev = settings(Some("p1"), vec![stored("p1", "A"), stored("p2", "B")]);
        let next = settings(
            Some("p1"),
            vec![
                stored("p1", "A"),
                with_open(stored("p2", "B"), "files.explorer"),
            ],
        );
        assert_eq!(transition(&prev, &next, false), LayoutTransition::None);
    }

    #[test]
    fn deleting_the_selected_preset_is_an_edit_not_a_switch() {
        let prev = settings(Some("p1"), vec![stored("p1", "A")]);
        let next = settings(None, vec![]);
        assert_eq!(transition(&prev, &next, false), LayoutTransition::Edit);
        let other = settings(Some("p2"), vec![stored("p2", "B")]);
        assert_eq!(
            transition(&prev, &other, false),
            LayoutTransition::Edit,
            "p was deleted in the same write that selected another"
        );
    }

    #[test]
    fn an_external_write_that_changes_nothing_is_none() {
        let same = settings(Some("p1"), vec![stored("p1", "A")]);
        assert_eq!(
            transition(&same, &same.clone(), false),
            LayoutTransition::None
        );
        assert_eq!(
            transition(&LayoutSettings::default(), &LayoutSettings::default(), true),
            LayoutTransition::None
        );
    }

    #[test]
    fn a_hand_edited_unknown_selection_acts_as_default() {
        let prev = settings(Some("gone"), vec![stored("p1", "A")]);
        let to_stored = settings(Some("p1"), vec![stored("p1", "A")]);
        assert_eq!(
            transition(&prev, &to_stored, false),
            LayoutTransition::Switch
        );
        let still_unknown = settings(Some("other"), vec![stored("p1", "A")]);
        assert_eq!(
            transition(&prev, &still_unknown, false),
            LayoutTransition::None
        );
    }

    #[test]
    fn the_legacy_flag_makes_an_unset_selection_swapped_in_both_documents() {
        let prev = settings(None, vec![]);
        let to_default = settings(Some("default"), vec![]);
        assert_eq!(
            transition(&prev, &to_default, true),
            LayoutTransition::Switch
        );
        assert_eq!(
            transition(&prev, &to_default, false),
            LayoutTransition::None
        );
    }
}

#[test]
fn settings_resolve_stored_presets_and_fall_back_to_default_for_unknown_ids() {
    use nocterm_plugin_model::{GroupSpec, PluginCatalog};
    let groups = [GroupSpec {
        id: GroupId::new("navigation"),
        title: "Sidebar",
        primary: true,
    }];
    let catalog = PluginCatalog::new(&groups, &[], &[]).expect("a catalog with one group");
    let one = settings(Some("p1"), vec![stored("p1", "A")]);
    assert_eq!(
        one.resolve(&pid("gone"), &catalog),
        BuiltIn::Default.resolve(&catalog)
    );
    assert_eq!(
        one.resolve(&pid("swapped"), &catalog),
        BuiltIn::Swapped.resolve(&catalog)
    );
    assert_eq!(
        one.resolve(&pid("p1"), &catalog),
        resolve(&one.presets[0], &catalog)
    );
}

#[test]
fn a_primary_flag_on_a_group_the_catalog_does_not_make_primary_is_reported_as_ignored() {
    use nocterm_plugin_model::{GroupSpec, PluginCatalog};
    let groups = [
        GroupSpec {
            id: GroupId::new("navigation"),
            title: "Sidebar",
            primary: true,
        },
        GroupSpec {
            id: GroupId::new("assistant"),
            title: "Assistant",
            primary: false,
        },
    ];
    let catalog = PluginCatalog::new(&groups, &[], &[]).expect("a catalog with two groups");
    let flagged = |group: &'static str| BarItem::Group {
        id: GroupId::new(group),
        side: None,
        primary: true,
        members: Vec::new(),
    };
    let mut preset = stored("p1", "A");
    preset.bar.left = vec![flagged("navigation"), flagged("assistant")];
    let layouts = settings(None, vec![preset]);

    assert_eq!(
        layouts.ignored_primary_flags(&catalog),
        [(pid("p1"), GroupId::new("assistant"))],
        "the catalog's own group may carry the flag"
    );
}

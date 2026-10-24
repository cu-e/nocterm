use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, GroupId, GroupSpec, Icon, PanelChrome,
    PanelTraits, Placement, PluginCatalog, Side, SystemContribution, WidthClass, Zone,
};

use super::*;

const fn contribution(
    id: &'static str,
    kind: ContributionKind,
    zone: Zone,
    order: u16,
    group: Option<&'static str>,
) -> SystemContribution {
    let panel = match kind {
        ContributionKind::Panel => Some(PanelTraits {
            width: WidthClass::Navigation,
            chrome: PanelChrome::Titled,
        }),
        ContributionKind::Widget => None,
    };
    SystemContribution {
        id: ContributionId::new(id),
        kind,
        title: "Title",
        icon: Icon::Layers,
        placement: Placement {
            zone,
            order,
            group,
            auto: AutoPlace::Show,
        },
        panel,
        preview: None,
        pinned: false,
    }
}

const fn panel(id: &'static str, order: u16) -> SystemContribution {
    contribution(
        id,
        ContributionKind::Panel,
        Zone::Left,
        order,
        Some("navigation"),
    )
}

const fn widget(id: &'static str, zone: Zone, order: u16) -> SystemContribution {
    contribution(id, ContributionKind::Widget, zone, order, None)
}

const fn assistant(id: &'static str, order: u16) -> SystemContribution {
    let mut agent = contribution(
        id,
        ContributionKind::Panel,
        Zone::Right,
        order,
        Some("assistant"),
    );
    agent.panel = Some(PanelTraits {
        width: WidthClass::Assistant,
        chrome: PanelChrome::OwnHeader,
    });
    agent
}

const fn pinned(mut widget: SystemContribution) -> SystemContribution {
    widget.pinned = true;
    widget
}

/// The shell as the application composes it today.
const SHELL: &[SystemContribution] = &[
    panel("shell.servers", 10),
    panel("shell.files", 20),
    panel("shell.snippets", 30),
    widget("shell.monitor", Zone::Left, 60),
    pinned(widget("shell.notices", Zone::Right, 10)),
    widget("shell.transfers", Zone::Right, 20),
    widget("shell.terminal", Zone::Right, 30),
    assistant("shell.agent", 40),
];

fn groups() -> [GroupSpec; 2] {
    [
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
    ]
}

fn catalog_of(core: &'static [SystemContribution]) -> PluginCatalog {
    PluginCatalog::new(&groups(), &[core], &[]).expect("a valid catalog")
}

fn catalog() -> PluginCatalog {
    catalog_of(SHELL)
}

fn id(id: &'static str) -> ContributionId {
    ContributionId::new(id)
}

fn preset(bar: Bar) -> LayoutPreset {
    LayoutPreset {
        bar,
        ..BuiltIn::preset()
    }
}

fn widget_item(name: &'static str) -> BarItem {
    BarItem::Item { id: id(name) }
}

fn group_item(name: &'static str, members: Vec<GroupEntry>) -> BarItem {
    BarItem::Group {
        id: GroupId::new(name),
        side: None,
        primary: name == "navigation",
        members,
    }
}

fn panels(names: &[&'static str]) -> Vec<GroupEntry> {
    names
        .iter()
        .map(|name| GroupEntry::Panel(id(name)))
        .collect()
}

/// A zone as short text: widgets by id, `|` for a separator, groups as `[id: members]`.
fn show(items: &[ResolvedItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            ResolvedItem::Widget(id) => id.to_string(),
            ResolvedItem::Separator => "|".to_owned(),
            ResolvedItem::Group(group) => {
                let members: Vec<_> = group
                    .members()
                    .iter()
                    .map(|member| match member {
                        ResolvedMember::Panel(id) => id.to_string(),
                        ResolvedMember::Separator => "|".to_owned(),
                    })
                    .collect();
                format!("[{}: {}]", group.id(), members.join(" "))
            }
        })
        .collect()
}

#[test]
fn the_default_places_everything_where_the_hints_put_it() {
    let layout = BuiltIn::Default.resolve(&catalog());

    assert_eq!(
        show(layout.zone(Zone::Left)),
        [
            "[navigation: shell.servers shell.files shell.snippets]",
            "shell.monitor"
        ]
    );
    assert!(layout.zone(Zone::Center).is_empty());
    assert_eq!(
        show(layout.zone(Zone::Right)),
        [
            "shell.notices",
            "shell.transfers",
            "shell.terminal",
            "[assistant: shell.agent]"
        ]
    );
    let sides: Vec<_> = layout
        .groups()
        .map(|group| (group.id().as_str().to_owned(), group.side()))
        .collect();
    assert_eq!(
        sides,
        [
            ("navigation".to_owned(), Side::Left),
            ("assistant".to_owned(), Side::Right)
        ]
    );
    assert!(layout.open().is_empty());
}

#[test]
fn swapped_flips_the_sides_and_nothing_else() {
    let default = BuiltIn::Default.resolve(&catalog());
    let swapped = BuiltIn::Swapped.resolve(&catalog());

    assert_ne!(default, swapped);
    for zone in [Zone::Left, Zone::Center, Zone::Right] {
        assert_eq!(show(default.zone(zone)), show(swapped.zone(zone)));
    }
    let sides = |layout: &ResolvedLayout| -> Vec<Side> {
        layout.groups().map(ResolvedGroup::side).collect()
    };
    assert_eq!(sides(&default), [Side::Left, Side::Right]);
    assert_eq!(sides(&swapped), [Side::Right, Side::Left]);
    assert_eq!(swapped.swapped(), default);
}

#[test]
fn built_ins_have_the_reserved_ids_and_are_found_by_them() {
    assert_eq!(BuiltIn::Default.id().as_str(), "default");
    assert_eq!(BuiltIn::Swapped.id().as_str(), "swapped");
    assert_eq!(
        BuiltIn::of(&PresetId::new("swapped")),
        Some(BuiltIn::Swapped)
    );
    assert_eq!(BuiltIn::of(&PresetId::new("p1")), None);
}

#[test]
fn unknown_ids_are_not_placed_and_the_preset_keeps_them() {
    let stored = preset(Bar {
        left: vec![
            widget_item("shell.missing"),
            group_item("navigation", panels(&["shell.files", "shell.gone"])),
            group_item("nowhere", panels(&["shell.servers"])),
        ],
        ..Bar::default()
    });
    let before = stored.clone();

    let layout = resolve(&stored, &catalog());

    assert_eq!(stored, before);
    assert!(!layout.is_placed(&id("shell.missing")));
    assert!(!layout.is_placed(&id("shell.gone")));
    assert_eq!(
        show(layout.zone(Zone::Left))[0],
        "[navigation: shell.files shell.servers shell.snippets]",
        "the files panel keeps its explicit place; the rest follow by their hints"
    );
}

#[test]
fn a_contribution_is_placed_once_and_a_wrong_kind_is_dropped() {
    let stored = preset(Bar {
        left: vec![
            group_item("navigation", panels(&["shell.files", "shell.monitor"])),
            widget_item("shell.files"),
        ],
        right: vec![
            widget_item("shell.terminal"),
            widget_item("shell.terminal"),
            group_item("assistant", panels(&["shell.files", "shell.agent"])),
        ],
        ..Bar::default()
    });

    let layout = resolve(&stored, &catalog());

    let placed: Vec<_> = layout
        .zone(Zone::Left)
        .iter()
        .chain(layout.zone(Zone::Right))
        .flat_map(|item| show(std::slice::from_ref(item)))
        .collect();
    assert_eq!(
        placed
            .iter()
            .filter(|text| text.contains("shell.files"))
            .count(),
        1
    );
    assert_eq!(
        placed
            .iter()
            .filter(|text| *text == "shell.terminal")
            .count(),
        1
    );
    assert!(
        layout.group_of(&id("shell.monitor")).is_none(),
        "a widget cannot be a group member"
    );
    assert!(
        layout.is_placed(&id("shell.monitor")),
        "it is placed by its hints"
    );
    assert!(
        layout
            .group_of(&id("shell.agent"))
            .is_some_and(|group| group.id().as_str() == "assistant")
    );
}

#[test]
fn separators_collapse_in_runs_and_at_the_ends_of_zones_and_groups() {
    let stored = preset(Bar {
        left: vec![
            BarItem::Separator,
            group_item(
                "navigation",
                vec![
                    GroupEntry::Separator,
                    GroupEntry::Panel(id("shell.servers")),
                    GroupEntry::Separator,
                    GroupEntry::Separator,
                    GroupEntry::Panel(id("shell.files")),
                    GroupEntry::Separator,
                ],
            ),
            BarItem::Separator,
            BarItem::Separator,
            widget_item("shell.monitor"),
            BarItem::Separator,
        ],
        ..Bar::default()
    });

    let layout = resolve(&stored, &catalog());

    assert_eq!(
        show(layout.zone(Zone::Left)),
        [
            "[navigation: shell.servers | shell.files | shell.snippets]",
            "|",
            "shell.monitor"
        ]
    );
}

#[test]
fn a_shown_contribution_missing_from_bar_and_tray_appears_once_at_its_hint() {
    let stored = preset(Bar {
        right: vec![widget_item("shell.terminal")],
        ..Bar::default()
    });

    let layout = resolve(&stored, &catalog());

    assert_eq!(
        show(layout.zone(Zone::Right)),
        [
            "shell.terminal",
            "shell.notices",
            "shell.transfers",
            "[assistant: shell.agent]"
        ]
    );
}

#[test]
fn a_tray_entry_stays_unplaced() {
    let stored = LayoutPreset {
        tray: vec![id("shell.transfers"), id("shell.snippets")],
        ..BuiltIn::preset()
    };

    let layout = resolve(&stored, &catalog());

    assert!(!layout.is_placed(&id("shell.transfers")));
    assert!(!layout.is_placed(&id("shell.snippets")));
    assert!(layout.is_placed(&id("shell.files")));
}

#[test]
fn a_tray_entry_that_the_bar_places_is_placed() {
    let stored = LayoutPreset {
        tray: vec![id("shell.transfers")],
        bar: Bar {
            left: vec![widget_item("shell.transfers")],
            ..Bar::default()
        },
        ..BuiltIn::preset()
    };

    let layout = resolve(&stored, &catalog());

    assert_eq!(show(layout.zone(Zone::Left))[0], "shell.transfers");
}

#[test]
fn a_contribution_that_does_not_ask_to_be_shown_waits_in_the_tray() {
    static QUIET: &[SystemContribution] = &[panel("quiet.files", 10), {
        let mut clock = widget("quiet.clock", Zone::Right, 20);
        clock.placement.auto = AutoPlace::Tray;
        clock
    }];

    let layout = BuiltIn::Default.resolve(&catalog_of(QUIET));

    assert!(layout.is_placed(&id("quiet.files")));
    assert!(!layout.is_placed(&id("quiet.clock")));
}

#[test]
fn a_panel_in_the_tray_is_hosted_by_the_group_its_hints_name() {
    let stored = LayoutPreset {
        tray: vec![id("shell.snippets"), id("shell.agent")],
        ..BuiltIn::preset()
    };

    let layout = resolve(&stored, &catalog());

    assert!(!layout.is_placed(&id("shell.snippets")));
    assert_eq!(
        layout.host_of(&id("shell.snippets")),
        Some(&GroupId::new("navigation"))
    );
    assert_eq!(
        layout.host_of(&id("shell.agent")),
        Some(&GroupId::new("assistant"))
    );
    assert_eq!(layout.host_of(&id("shell.monitor")), None, "a widget");
    assert_eq!(layout.host_of(&id("shell.unknown")), None);
}

#[test]
fn a_placed_panel_is_hosted_by_the_group_that_places_it() {
    let stored = preset(Bar {
        right: vec![group_item("assistant", panels(&["shell.files"]))],
        ..Bar::default()
    });

    let layout = resolve(&stored, &catalog());

    assert_eq!(
        layout.host_of(&id("shell.files")),
        Some(&GroupId::new("assistant")),
        "the preset moved the panel out of the group its hints name"
    );
    assert_eq!(
        layout.host_of(&id("shell.servers")),
        Some(&GroupId::new("navigation"))
    );
}

#[test]
fn a_pinned_contribution_cannot_leave_the_bar() {
    let stored = LayoutPreset {
        tray: vec![id("shell.notices")],
        bar: Bar {
            right: vec![widget_item("shell.terminal")],
            ..Bar::default()
        },
        ..BuiltIn::preset()
    };
    let layout = resolve(&stored, &catalog());

    assert!(layout.is_placed(&id("shell.notices")));
    let right = show(layout.zone(Zone::Right));
    assert_eq!(right.last().map(String::as_str), Some("shell.notices"));
}

#[test]
fn the_primary_group_is_in_the_bar_even_without_members() {
    static NO_NAVIGATION: &[SystemContribution] = &[
        widget("bare.clock", Zone::Right, 10),
        assistant("bare.agent", 20),
    ];

    let layout = BuiltIn::Default.resolve(&catalog_of(NO_NAVIGATION));

    assert_eq!(
        show(layout.zone(Zone::Left)),
        ["[navigation: ]"],
        "an empty sidebar"
    );
    assert!(
        layout
            .primary_group()
            .is_some_and(|group| group.id().as_str() == "navigation")
    );
}

#[test]
fn the_catalog_decides_which_group_is_primary_whatever_the_preset_flags() {
    let stored = preset(Bar {
        left: vec![BarItem::Group {
            id: GroupId::new("assistant"),
            side: None,
            primary: true,
            members: panels(&["shell.agent"]),
        }],
        right: vec![BarItem::Group {
            id: GroupId::new("navigation"),
            side: Some(Side::Left),
            primary: false,
            members: panels(&["shell.servers"]),
        }],
        ..Bar::default()
    });

    let layout = resolve(&stored, &catalog());

    let primaries: Vec<_> = layout
        .groups()
        .filter(|group| group.is_primary())
        .map(|group| group.id().as_str().to_owned())
        .collect();
    assert_eq!(
        primaries,
        ["navigation"],
        "the flag in the preset is not read"
    );
}

#[test]
fn a_group_other_than_the_primary_one_needs_a_panel() {
    let stored = preset(Bar {
        right: vec![group_item("assistant", Vec::new())],
        ..Bar::default()
    });
    let tray = LayoutPreset {
        tray: vec![id("shell.agent")],
        ..stored
    };

    let layout = resolve(&tray, &catalog());

    assert!(
        layout
            .groups()
            .all(|group| group.id().as_str() == "navigation")
    );
}

#[test]
fn a_group_takes_the_side_of_its_zone_unless_the_preset_says_otherwise() {
    let stored = preset(Bar {
        left: vec![BarItem::Group {
            id: GroupId::new("navigation"),
            side: Some(Side::Right),
            primary: true,
            members: panels(&["shell.servers"]),
        }],
        center: vec![group_item("assistant", panels(&["shell.agent"]))],
        ..Bar::default()
    });

    let layout = resolve(&stored, &catalog());

    let sides: Vec<_> = layout.groups().map(ResolvedGroup::side).collect();
    assert_eq!(
        sides,
        [Side::Right, Side::Left],
        "a centre group defaults to the left"
    );
}

#[test]
fn open_keeps_the_first_placed_panel_of_each_group() {
    let stored = LayoutPreset {
        open: vec![
            id("shell.files"),
            id("shell.servers"),
            id("shell.agent"),
            id("shell.terminal"),
            id("shell.unknown"),
        ],
        ..BuiltIn::preset()
    };

    let layout = resolve(&stored, &catalog());

    assert_eq!(
        layout.open(),
        [id("shell.files"), id("shell.agent"), id("shell.terminal")]
    );
}

#[test]
fn a_panel_without_a_hint_joins_the_group_of_its_width_class() {
    static LOOSE: &[SystemContribution] = &[
        contribution("loose.list", ContributionKind::Panel, Zone::Left, 10, None),
        {
            let mut chat = assistant("loose.chat", 20);
            chat.placement.group = None;
            chat
        },
    ];

    let layout = BuiltIn::Default.resolve(&catalog_of(LOOSE));

    let group = |panel: &'static str| {
        layout
            .group_of(&id(panel))
            .map(|group| group.id().as_str().to_owned())
    };
    assert_eq!(group("loose.list").as_deref(), Some("navigation"));
    assert_eq!(group("loose.chat").as_deref(), Some("assistant"));
}

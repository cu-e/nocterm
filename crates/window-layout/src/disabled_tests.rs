//! Disabled plugins in a resolved layout: kept in the preset, absent from what is drawn.

use std::collections::BTreeSet;

use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, GroupId, GroupSpec, Icon, PanelChrome,
    PanelTraits, Placement, PluginCatalog, PluginId, PluginSet, SettingsSchema, SystemContribution,
    SystemPlugin, WidthClass, Zone,
};

use crate::*;

const fn entry(
    id: &'static str,
    kind: ContributionKind,
    zone: Zone,
    order: u16,
    group: Option<&'static str>,
) -> SystemContribution {
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
        panel: match kind {
            ContributionKind::Panel => Some(PanelTraits {
                width: WidthClass::Navigation,
                chrome: PanelChrome::Titled,
            }),
            ContributionKind::Widget => None,
        },
        preview: None,
        pinned: false,
    }
}

const CORE: &[SystemContribution] = &[
    entry(
        "shell.servers",
        ContributionKind::Panel,
        Zone::Left,
        5,
        Some("navigation"),
    ),
    entry(
        "shell.notices",
        ContributionKind::Widget,
        Zone::Right,
        5,
        None,
    ),
];

static TOOLS: SystemPlugin = SystemPlugin {
    id: PluginId::new("tools"),
    name: "Tools",
    description: "Panels and widgets that can be switched off",
    required: false,
    contributions: &[
        entry(
            "tools.list",
            ContributionKind::Panel,
            Zone::Left,
            10,
            Some("navigation"),
        ),
        entry(
            "tools.extra",
            ContributionKind::Panel,
            Zone::Left,
            20,
            Some("navigation"),
        ),
        entry(
            "tools.clock",
            ContributionKind::Widget,
            Zone::Right,
            10,
            None,
        ),
        {
            let mut pin = entry("tools.pin", ContributionKind::Widget, Zone::Right, 30, None);
            pin.pinned = true;
            pin
        },
    ],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: None,
};

fn catalog() -> PluginSet {
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
    PluginCatalog::new(&groups, &[CORE], &[&TOOLS])
        .expect("a valid catalog")
        .into()
}

fn tools_off() -> BTreeSet<PluginId> {
    BTreeSet::from([PluginId::new("tools")])
}

fn id(id: &'static str) -> ContributionId {
    ContributionId::new(id)
}

fn widget(name: &'static str) -> BarItem {
    BarItem::Item { id: id(name) }
}

fn navigation(members: Vec<GroupEntry>) -> BarItem {
    BarItem::Group {
        id: GroupId::new("navigation"),
        side: None,
        primary: true,
        members,
    }
}

fn panel(name: &'static str) -> GroupEntry {
    GroupEntry::Panel(id(name))
}

fn preset(bar: Bar) -> LayoutPreset {
    LayoutPreset {
        bar,
        ..BuiltIn::preset()
    }
}

fn widgets(items: &[ResolvedItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            ResolvedItem::Widget(id) => id.to_string(),
            ResolvedItem::Separator => "|".to_owned(),
            ResolvedItem::Group(group) => format!("[{}]", group.id()),
        })
        .collect()
}

fn members(layout: &ResolvedLayout) -> Vec<String> {
    layout
        .primary_group()
        .map(|group| group.panels().map(ToString::to_string).collect())
        .unwrap_or_default()
}

#[test]
fn a_disabled_plugin_is_left_out_of_the_default_and_the_tray() {
    let catalog = catalog();
    let on = BuiltIn::Default.resolve(&catalog, &BTreeSet::new());
    let off = BuiltIn::Default.resolve(&catalog, &tools_off());

    assert_eq!(members(&on), ["shell.servers", "tools.list", "tools.extra"]);
    assert_eq!(members(&off), ["shell.servers"]);
    assert_eq!(widgets(off.zone(Zone::Right)), ["shell.notices"]);
    assert!(!off.is_placed(&id("tools.pin")), "a pinned widget too");
    assert!(off.host_of(&id("tools.list")).is_none());
    assert!(off.disabled().is_empty(), "the default names nothing");
}

#[test]
fn a_disabled_entry_stays_in_the_preset_but_is_not_drawn() {
    let stored = preset(Bar {
        left: vec![navigation(vec![
            panel("tools.list"),
            panel("shell.servers"),
        ])],
        right: vec![widget("tools.clock"), widget("tools.clock")],
        ..Bar::default()
    });
    let before = stored.clone();

    let off = resolve(&stored, &catalog(), &tools_off());

    assert_eq!(stored, before);
    assert_eq!(members(&off), ["shell.servers"]);
    assert!(!off.is_placed(&id("tools.clock")));
    assert_eq!(off.disabled(), [id("tools.list"), id("tools.clock")]);
}

#[test]
fn a_disabled_plugin_is_not_added_back_by_its_hints() {
    let stored = preset(Bar {
        left: vec![navigation(vec![panel("shell.servers")])],
        ..Bar::default()
    });

    let on = resolve(&stored, &catalog(), &BTreeSet::new());
    let off = resolve(&stored, &catalog(), &tools_off());

    assert_eq!(members(&on), ["shell.servers", "tools.list", "tools.extra"]);
    assert_eq!(members(&off), ["shell.servers"]);
    assert!(
        off.disabled().is_empty(),
        "only the bar's own entries count"
    );
}

#[test]
fn a_disabled_panel_is_not_opened_and_the_next_one_of_its_group_is() {
    let stored = LayoutPreset {
        open: vec![id("tools.list"), id("shell.servers")],
        ..preset(Bar {
            left: vec![navigation(vec![
                panel("tools.list"),
                panel("shell.servers"),
            ])],
            ..Bar::default()
        })
    };

    assert_eq!(
        resolve(&stored, &catalog(), &BTreeSet::new()).open(),
        [id("tools.list")]
    );
    assert_eq!(
        resolve(&stored, &catalog(), &tools_off()).open(),
        [id("shell.servers")]
    );
}

#[test]
fn separators_collapse_around_a_disabled_entry() {
    let stored = preset(Bar {
        left: vec![navigation(vec![
            panel("shell.servers"),
            GroupEntry::Separator,
            panel("tools.list"),
            GroupEntry::Separator,
            panel("tools.extra"),
        ])],
        right: vec![
            widget("shell.notices"),
            BarItem::Separator,
            widget("tools.clock"),
            BarItem::Separator,
        ],
        ..Bar::default()
    });

    let off = resolve(&stored, &catalog(), &tools_off());

    let group = off.primary_group().expect("the primary group");
    assert_eq!(group.members().len(), 1);
    assert_eq!(widgets(off.zone(Zone::Right)), ["shell.notices"]);
}

#[test]
fn a_group_left_with_no_enabled_panel_leaves_the_bar_unless_it_is_primary() {
    static ASSISTANT: SystemPlugin = SystemPlugin {
        id: PluginId::new("chat"),
        name: "Chat",
        description: "An assistant panel",
        required: false,
        contributions: &[entry(
            "chat.agent",
            ContributionKind::Panel,
            Zone::Right,
            40,
            Some("assistant"),
        )],
        settings_section: None,
        settings: SettingsSchema::NONE,
        settings_page: None,
    };
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
    let catalog =
        PluginSet::from(PluginCatalog::new(&groups, &[CORE], &[&TOOLS, &ASSISTANT]).unwrap());
    let off = BTreeSet::from([PluginId::new("chat"), PluginId::new("tools")]);

    let layout = BuiltIn::Default.resolve(&catalog, &off);

    let ids: Vec<_> = layout
        .groups()
        .map(|group| group.id().to_string())
        .collect();
    assert_eq!(ids, ["navigation"]);
    assert!(layout.host_of(&id("chat.agent")).is_none());
}

#[test]
fn the_swapped_layout_keeps_the_list_of_disabled_entries() {
    let stored = preset(Bar {
        left: vec![navigation(vec![panel("tools.list")])],
        ..Bar::default()
    });

    let layout = resolve(&stored, &catalog(), &tools_off());

    assert_eq!(layout.swapped().disabled(), layout.disabled());
}

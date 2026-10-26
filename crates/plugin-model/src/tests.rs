use super::*;
use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec,
    vec::Vec,
};

const fn widget(id: &'static str, zone: Zone, order: u16) -> SystemContribution {
    SystemContribution {
        id: ContributionId::new(id),
        kind: ContributionKind::Widget,
        title: "Widget",
        icon: Icon::Clock,
        placement: Placement {
            zone,
            order,
            group: None,
            auto: AutoPlace::Show,
        },
        panel: None,
        preview: None,
        pinned: false,
    }
}

const fn panel(id: &'static str, order: u16, group: Option<&'static str>) -> SystemContribution {
    SystemContribution {
        id: ContributionId::new(id),
        kind: ContributionKind::Panel,
        title: "Panel",
        icon: Icon::Layers,
        placement: Placement {
            zone: Zone::Left,
            order,
            group,
            auto: AutoPlace::Show,
        },
        panel: Some(PanelTraits {
            width: WidthClass::Navigation,
            chrome: PanelChrome::Titled,
        }),
        preview: None,
        pinned: false,
    }
}

const FIRST: SystemContribution = panel("alpha.first", 10, Some("navigation"));
const SECOND: SystemContribution = widget("alpha.second", Zone::Right, 10);

// A plugin is static data: a slice of descriptors with borrowed ids needs no allocation. It is a
// `static` rather than a `const`, because a reference to a constant that holds a `Cow` is not
// promoted to `'static`.
static ALPHA: SystemPlugin = SystemPlugin {
    id: PluginId::new("alpha"),
    name: "Alpha",
    description: "The first test plugin",
    required: false,
    contributions: &[FIRST, SECOND],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: None,
};
static BETA: SystemPlugin = SystemPlugin {
    id: PluginId::new("beta"),
    name: "Beta",
    description: "The second test plugin",
    required: true,
    contributions: &[widget("beta.clock", Zone::Right, 20)],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: None,
};
const CORE: &[SystemContribution] = &[widget("shell.notice", Zone::Right, 5)];

fn groups() -> Vec<GroupSpec> {
    vec![GroupSpec {
        id: GroupId::new("navigation"),
        title: "Sidebar",
        primary: true,
    }]
}

#[test]
fn a_catalog_lists_core_and_plugin_contributions_in_zone_order() {
    let catalog = PluginCatalog::new(&groups(), &[CORE], &[&ALPHA, &BETA]).unwrap();
    let ids: Vec<_> = catalog.entries().map(|entry| entry.id.as_str()).collect();
    assert_eq!(
        ids,
        ["alpha.first", "shell.notice", "alpha.second", "beta.clock"]
    );
    let plugins: Vec<_> = catalog.plugins().map(|plugin| plugin.id.as_str()).collect();
    assert_eq!(plugins, ["alpha", "beta"]);
}

#[test]
fn ownership_tells_core_from_plugin_contributions() {
    let catalog = PluginCatalog::new(&groups(), &[CORE], &[&ALPHA, &BETA]).unwrap();
    let core = ContributionId::new("shell.notice");
    let owned = ContributionId::new("beta.clock");
    assert!(matches!(catalog.owner(&core), Some(Owner::Core)));
    assert!(catalog.plugin_of(&core).is_none());
    assert!(matches!(catalog.owner(&owned), Some(Owner::Plugin(plugin)) if plugin.required));
    assert_eq!(
        catalog.plugin_of(&owned).map(|plugin| plugin.name),
        Some("Beta")
    );
    assert!(
        catalog
            .owner(&ContributionId::new("beta.missing"))
            .is_none()
    );
}

#[test]
fn a_duplicate_plugin_is_refused() {
    let error = PluginCatalog::new(&groups(), &[], &[&ALPHA, &ALPHA]).err();
    assert_eq!(
        error,
        Some(CatalogError::DuplicatePlugin(PluginId::new("alpha")))
    );
}

#[test]
fn a_duplicate_contribution_is_refused() {
    const TWIN: SystemContribution = widget("shell.notice", Zone::Left, 1);
    const TWINS: &[SystemContribution] = &[TWIN];
    let error = PluginCatalog::new(&groups(), &[CORE, TWINS], &[]).err();
    assert_eq!(
        error,
        Some(CatalogError::Duplicate(ContributionId::new("shell.notice")))
    );
}

#[test]
fn a_duplicate_group_is_refused() {
    let mut twice = groups();
    twice.extend(groups());
    let error = PluginCatalog::new(&twice, &[], &[]).err();
    assert_eq!(
        error,
        Some(CatalogError::DuplicateGroup(GroupId::new("navigation")))
    );
}

#[test]
fn a_contribution_outside_its_plugin_namespace_is_refused() {
    static STRAY: SystemPlugin = SystemPlugin {
        id: PluginId::new("gamma"),
        name: "Gamma",
        description: "",
        required: false,
        contributions: &[widget("delta.clock", Zone::Left, 1)],
        settings_section: None,
        settings: SettingsSchema::NONE,
        settings_page: None,
    };
    let error = PluginCatalog::new(&groups(), &[], &[&STRAY]).err();
    assert_eq!(
        error,
        Some(CatalogError::Namespace {
            id: ContributionId::new("delta.clock"),
            expected: PluginId::new("gamma"),
        })
    );
}

#[test]
fn a_core_contribution_inside_a_plugin_namespace_is_refused() {
    const NOISY: &[SystemContribution] = &[widget("alpha.core", Zone::Left, 1)];
    let error = PluginCatalog::new(&groups(), &[NOISY], &[&ALPHA]).err();
    assert_eq!(
        error,
        Some(CatalogError::CoreNamespace {
            id: ContributionId::new("alpha.core"),
            plugin: PluginId::new("alpha"),
        })
    );
}

#[test]
fn a_group_hint_must_name_a_declared_group() {
    const HINTED: &[SystemContribution] = &[panel("shell.side", 1, Some("sideways"))];
    let error = PluginCatalog::new(&groups(), &[HINTED], &[]).err();
    assert_eq!(
        error,
        Some(CatalogError::UnknownGroup {
            id: ContributionId::new("shell.side"),
            group: "sideways".to_string(),
        })
    );
}

#[test]
fn a_widget_cannot_name_a_group() {
    let hinted = SystemContribution {
        placement: Placement {
            group: Some("navigation"),
            ..widget("shell.hinted", Zone::Left, 1).placement
        },
        ..widget("shell.hinted", Zone::Left, 1)
    };
    let core: &'static [SystemContribution] = Box::leak(Box::new([hinted]));
    assert_eq!(
        PluginCatalog::new(&groups(), &[core], &[]).err(),
        Some(CatalogError::WidgetGroup {
            id: ContributionId::new("shell.hinted"),
            group: "navigation".to_string(),
        })
    );
}

#[test]
fn exactly_one_group_is_primary() {
    let group = |id, primary| GroupSpec {
        id: GroupId::new(id),
        title: "Group",
        primary,
    };
    let none = vec![group("first", false), group("second", false)];
    let two = vec![group("first", true), group("second", true)];
    for (groups, count) in [(Vec::new(), 0), (none, 0), (two, 2)] {
        assert_eq!(
            PluginCatalog::new(&groups, &[], &[]).err(),
            Some(CatalogError::PrimaryGroups { count })
        );
    }
}

#[test]
fn panel_traits_exist_exactly_for_panels() {
    let bare = SystemContribution {
        panel: None,
        ..panel("shell.bare", 1, None)
    };
    let spurious = SystemContribution {
        panel: Some(PanelTraits {
            width: WidthClass::Assistant,
            chrome: PanelChrome::OwnHeader,
        }),
        ..widget("shell.spurious", Zone::Left, 2)
    };
    for contribution in [bare, spurious] {
        let id = contribution.id.clone();
        let core: &'static [SystemContribution] = Box::leak(Box::new([contribution]));
        let error = PluginCatalog::new(&groups(), &[core], &[]).err();
        assert_eq!(error, Some(CatalogError::PanelTraits { id }));
    }
}

#[test]
fn order_values_are_unique_within_a_zone_only() {
    const CLASH: &[SystemContribution] = &[widget("shell.clash", Zone::Right, 10)];
    let error = PluginCatalog::new(&groups(), &[CLASH], &[&ALPHA]).err();
    assert_eq!(
        error,
        Some(CatalogError::DuplicateOrder {
            zone: Zone::Right,
            order: 10
        })
    );
    // The same order in another zone is fine.
    const OTHER: &[SystemContribution] = &[widget("shell.other", Zone::Center, 10)];
    assert!(PluginCatalog::new(&groups(), &[OTHER], &[&ALPHA]).is_ok());
}

#[test]
fn malformed_ids_are_refused_with_the_matching_variant() {
    const SHOUTING: &[SystemContribution] = &[widget("Shell.notice", Zone::Left, 1)];
    const NO_DOT: &[SystemContribution] = &[widget("shellnotice", Zone::Left, 1)];
    const LONG: &[SystemContribution] = &[widget(
        "shell.abcdefghijklmnopqrstuvwxyzabcdefghij",
        Zone::Left,
        1,
    )];
    let refused = |core| match PluginCatalog::new(&groups(), &[core], &[]) {
        Err(CatalogError::InvalidId(error)) => error,
        other => panic!("expected an id error, got {other:?}"),
    };
    assert!(matches!(refused(SHOUTING), IdError::Syntax { .. }));
    assert!(matches!(refused(NO_DOT), IdError::Syntax { .. }));
    assert!(matches!(refused(LONG), IdError::TooLong { .. }));
    assert_eq!(
        PluginId::new("").check(),
        Err(IdError::Empty { id: String::new() })
    );
    // The whole id is reported when only one part of it is empty.
    const NO_NAME: &[SystemContribution] = &[widget("shell.", Zone::Left, 1)];
    assert_eq!(
        refused(NO_NAME),
        IdError::Empty {
            id: "shell.".to_string()
        }
    );
}

#[test]
fn an_invalid_plugin_id_is_refused() {
    static BAD: SystemPlugin = SystemPlugin {
        id: PluginId::new("bad_id"),
        name: "Bad",
        description: "",
        required: false,
        contributions: &[],
        settings_section: None,
        settings: SettingsSchema::NONE,
        settings_page: None,
    };
    let error = PluginCatalog::new(&groups(), &[], &[&BAD]).err();
    assert!(matches!(
        error,
        Some(CatalogError::InvalidId(IdError::Syntax { .. }))
    ));
}

#[test]
fn catalog_errors_keep_the_id_error_as_their_source() {
    use std::error::Error as _;
    let cause = IdError::Empty {
        id: "shell.".to_string(),
    };
    let error = CatalogError::from(cause.clone());
    assert_eq!(
        error.source().map(ToString::to_string),
        Some(cause.to_string())
    );
}

#[test]
fn a_contribution_namespace_is_the_part_before_the_dot() {
    assert_eq!(ContributionId::new("files.explorer").namespace(), "files");
    assert_eq!(ContributionId::new("nodot").namespace(), "nodot");
}

#[test]
fn swapping_sides_flips_only_when_asked() {
    assert_eq!(Side::Left.swapped_if(false), Side::Left);
    assert_eq!(Side::Left.swapped_if(true), Side::Right);
    assert_eq!(Side::Right.swapped_if(true), Side::Left);
}

static USERISH: SystemPlugin = SystemPlugin {
    id: PluginId::new("user-acme-clock"),
    name: "Spoof",
    description: "A system plugin that squats the user space",
    required: false,
    contributions: &[widget("user-acme-clock.time", Zone::Right, 40)],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: None,
};

#[test]
fn the_user_prefix_is_reserved_for_plugins_and_core_namespaces() {
    let error = PluginCatalog::new(&groups(), &[], &[&USERISH]).err();
    assert_eq!(
        error,
        Some(CatalogError::Reserved {
            id: "user-acme-clock".to_string()
        })
    );
    const SQUAT: &[SystemContribution] = &[widget("user-acme-clock.time", Zone::Left, 1)];
    let error = PluginCatalog::new(&groups(), &[SQUAT], &[]).err();
    assert_eq!(
        error,
        Some(CatalogError::Reserved {
            id: "user-acme-clock.time".to_string()
        })
    );
    assert!(PluginCatalog::new(&groups(), &[CORE], &[&ALPHA, &BETA]).is_ok());
}

#[test]
fn a_user_plugin_id_is_built_from_publisher_and_name() {
    let id = PluginId::user("acme", "clock2").unwrap();
    assert_eq!(id.as_str(), "user-acme-clock2");
    assert!(id.is_user() && id.check().is_ok());
    assert!(!PluginId::new("monitor").is_user());
    for (publisher, name) in [("Acme", "clock"), ("acme", "my-clock"), ("ac me", "x")] {
        assert!(matches!(
            PluginId::user(publisher, name),
            Err(IdError::Syntax { .. })
        ));
    }
    assert!(matches!(
        PluginId::user("", "clock"),
        Err(IdError::Empty { .. })
    ));
    assert!(matches!(
        PluginId::user("publisherpublisher", "namenamename"),
        Err(IdError::TooLong { .. })
    ));
}

#[test]
fn a_user_contribution_id_joins_the_plugin_id_and_a_local_name() {
    let plugin = PluginId::user("acme", "clock").unwrap();
    let id = ContributionId::in_plugin(&plugin, "time").unwrap();
    assert_eq!(id.as_str(), "user-acme-clock.time");
    assert_eq!(id.namespace(), plugin.as_str());
    for local in ["", "Time", "a.b", "with space"] {
        assert!(ContributionId::in_plugin(&plugin, local).is_err());
    }
}

/// The position of an icon in `Icon::ALL`. The match is exhaustive, so a new icon does not
/// compile until it is placed here, and the test below then checks the list.
const fn position(icon: Icon) -> usize {
    match icon {
        Icon::Activity => 0,
        Icon::Bell => 1,
        Icon::Bot => 2,
        Icon::Box => 3,
        Icon::Clock => 4,
        Icon::Container => 5,
        Icon::FolderTree => 6,
        Icon::Gem => 7,
        Icon::Hexagon => 8,
        Icon::Layers => 9,
        Icon::PanelRight => 10,
        Icon::Plug => 11,
        Icon::Puzzle => 12,
        Icon::ScrollText => 13,
        Icon::Server => 14,
        Icon::Shapes => 15,
        Icon::ShieldCheck => 16,
        Icon::SquareTerminal => 17,
        Icon::Upload => 18,
    }
}

#[test]
fn the_icon_list_names_every_icon_once_in_order() {
    for (index, icon) in Icon::ALL.into_iter().enumerate() {
        assert_eq!(position(icon), index, "{icon:?}");
    }
    assert_eq!(position(Icon::Upload) + 1, Icon::ALL.len());
}

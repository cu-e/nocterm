use super::*;
use crate::schema::fixtures::{field, group, schema_of};
use crate::{FieldKind, PluginRef};
use alloc::{format, string::ToString, vec};

fn plugin_id() -> PluginId {
    PluginId::user("acme", "weather").unwrap()
}

fn clamped(local: &str, title: &str, icon: Icon) -> UserContribution {
    UserContribution::clamp(
        ContributionId::in_plugin(&plugin_id(), local).unwrap(),
        ContributionKind::Widget,
        title,
        icon,
        Zone::Right,
        7,
    )
    .unwrap()
}

#[test]
fn the_clamp_strips_every_built_in_privilege() {
    let widget = clamped("temp", "Temperature", Icon::Activity);
    let placement = widget.placement();
    assert_eq!(placement.auto, AutoPlace::Tray);
    assert_eq!(placement.group, None);
    assert_eq!((placement.zone, placement.order), (Zone::Right, 7));
    assert!(!widget.pinned() && !widget.expandable());
    assert_eq!(widget.chrome(), None);
    let panel = UserContribution::clamp(
        ContributionId::in_plugin(&plugin_id(), "forecast").unwrap(),
        ContributionKind::Panel,
        "Forecast",
        Icon::Layers,
        Zone::Left,
        1,
    )
    .unwrap();
    assert_eq!(panel.chrome(), Some(PanelChrome::Titled));
    assert_eq!(panel.placement().group, None);
}

#[test]
fn the_clamp_replaces_host_icons_and_sanitizes_the_title() {
    for icon in Icon::ALL.into_iter().filter(|icon| !icon.user_allowed()) {
        assert_eq!(clamped("a", "A", icon).icon(), Icon::Plug, "{icon:?}");
    }
    for icon in Icon::ALL.into_iter().filter(|icon| icon.user_allowed()) {
        assert_eq!(clamped("a", "A", icon).icon(), icon, "{icon:?}");
    }
    let spoofed = clamped("a", "Sys\u{202e}tem\u{200b}\nAlert", Icon::Plug);
    assert_eq!(spoofed.title(), "SystemAlert");
    let long = clamped("a", &"x".repeat(200), Icon::Plug);
    assert_eq!(long.title().chars().count(), 48);
}

#[test]
fn the_clamp_refuses_an_empty_title_and_ids_outside_the_user_space() {
    let id = ContributionId::in_plugin(&plugin_id(), "a").unwrap();
    let blank = UserContribution::clamp(
        id,
        ContributionKind::Widget,
        " \u{200b}\u{202e} ",
        Icon::Plug,
        Zone::Left,
        0,
    );
    assert_eq!(blank, Err(UserPluginError::Empty { field: "title" }));
    let system = UserContribution::clamp(
        ContributionId::new("monitor.summary"),
        ContributionKind::Widget,
        "Monitor",
        Icon::Plug,
        Zone::Left,
        0,
    );
    assert!(matches!(system, Err(UserPluginError::NotUserSpace { .. })));
    let malformed = UserContribution::clamp(
        ContributionId::new("user-acme-weather"),
        ContributionKind::Widget,
        "x",
        Icon::Plug,
        Zone::Left,
        0,
    );
    assert!(matches!(malformed, Err(UserPluginError::Id(_))));
}

fn build(
    id: PluginId,
    contributions: Vec<UserContribution>,
    settings: SettingsSchema,
) -> Result<UserPlugin, UserPluginError> {
    UserPlugin::new(id, "Weather", "Shows the weather", contributions, settings)
}

#[test]
fn a_user_plugin_joins_its_contributions_and_schema() {
    let schema = SettingsSchema {
        version: 2,
        groups: vec![group(
            "City",
            Some("user-acme-weather.temp"),
            vec![field("city", FieldKind::Toggle { default: true })],
        )]
        .into(),
    };
    let plugin = build(
        plugin_id(),
        vec![clamped("temp", "Temp", Icon::Plug)],
        schema,
    )
    .unwrap();
    assert_eq!(plugin.name(), "Weather");
    assert_eq!(plugin.contributions().len(), 1);
    assert_eq!(plugin.settings().version, 2);
}

#[test]
fn a_user_plugin_is_refused_outside_the_user_space_or_with_foreign_contributions() {
    assert!(matches!(
        build(PluginId::new("monitor"), vec![], SettingsSchema::NONE),
        Err(UserPluginError::NotUserSpace { .. })
    ));
    let other = PluginId::user("evil", "twin").unwrap();
    let foreign = UserContribution::clamp(
        ContributionId::in_plugin(&other, "x").unwrap(),
        ContributionKind::Widget,
        "x",
        Icon::Plug,
        Zone::Left,
        0,
    )
    .unwrap();
    assert!(matches!(
        build(plugin_id(), vec![foreign], SettingsSchema::NONE),
        Err(UserPluginError::Namespace { .. })
    ));
    let twice = vec![
        clamped("temp", "A", Icon::Plug),
        clamped("temp", "B", Icon::Plug),
    ];
    assert!(matches!(
        build(plugin_id(), twice, SettingsSchema::NONE),
        Err(UserPluginError::Duplicate(_))
    ));
    let unknown_group = SettingsSchema {
        version: 1,
        groups: vec![group("G", Some("user-acme-weather.nope"), vec![])].into(),
    };
    assert!(matches!(
        build(plugin_id(), vec![], unknown_group),
        Err(UserPluginError::Schema(_))
    ));
    assert!(matches!(
        UserPlugin::new(plugin_id(), "\u{200b}", "", vec![], schema_of(vec![])),
        Err(UserPluginError::Empty { field: "name" })
    ));
}

#[test]
fn the_description_is_sanitized_and_bounded() {
    let plugin = UserPlugin::new(
        plugin_id(),
        "Weather",
        &format!("Line\none\u{202e}{}", "y".repeat(400)),
        vec![],
        SettingsSchema::NONE,
    )
    .unwrap();
    assert!(plugin.description().starts_with("Lineone"));
    assert_eq!(plugin.description().chars().count(), 160);
}

#[test]
fn read_views_treat_both_tiers_alike_without_granting_privileges() {
    let plugin = build(
        plugin_id(),
        vec![clamped("temp", "Temp", Icon::Plug)],
        SettingsSchema::NONE,
    )
    .unwrap();
    let view = PluginRef::from(&plugin);
    assert_eq!(view.tier(), PluginTier::User);
    assert!(!view.required());
    assert_eq!(view.id().to_string(), "user-acme-weather");
    let items: Vec<_> = view.contributions().collect();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].tier(), PluginTier::User);
    assert!(!items[0].pinned());
    assert_eq!(items[0].placement().auto, AutoPlace::Tray);
    assert_eq!(items[0].title(), "Temp");
}

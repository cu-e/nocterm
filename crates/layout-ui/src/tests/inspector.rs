//! The inspector shows where the selected item comes from and the settings it has.

use std::{cell::RefCell, rc::Rc};

use gpui_kit::TestAppContext;
use nocterm_plugin_enablement::PluginEnablement;
use nocterm_plugin_model::{FieldEdit, GroupId, SettingValue, Side};
use nocterm_ui::PluginSettingsSource as _;
use nocterm_window_layout::{Bar, BarItem, LayoutPreset, LayoutSettings, PresetId};
use nocterm_workspace::OpenPlugins;

use super::{catalog, open};
use crate::snapshot::Snapshot;

const SECONDS: &str = "plugin-setting:tools:seconds:";

fn opened_plugins(cx: &mut TestAppContext) -> Rc<RefCell<Vec<Option<String>>>> {
    let opened = Rc::new(RefCell::new(Vec::new()));
    let sink = opened.clone();
    cx.update(|cx| {
        cx.on_action(move |action: &OpenPlugins, _| {
            sink.borrow_mut()
                .push(action.plugin.as_ref().map(ToString::to_string));
        });
    });
    opened
}

#[gpui_kit::test]
fn a_core_item_is_built_in_and_has_no_settings(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-item:shell.transfers", cx);

    assert_eq!(
        fixture.label("layout-owner", cx).as_deref(),
        Some("Built in")
    );
    assert!(fixture.has("layout-controls", cx));
    assert!(fixture.has("layout-inspector:shell.transfers", cx));
}

#[gpui_kit::test]
fn an_item_of_a_plugin_without_settings_links_to_the_plugin(cx: &mut TestAppContext) {
    let fixture = open(cx);
    let opened = opened_plugins(cx);
    fixture.click("layout-item:plain.note", cx);

    assert_eq!(
        fixture.label("layout-owner", cx).as_deref(),
        Some("Part of Plain")
    );
    assert!(fixture.has("layout-no-settings", cx));
    fixture.click("layout-manage-plugin", cx);
    assert_eq!(*opened.borrow(), [Some("plain".to_owned())]);
}

#[gpui_kit::test]
fn the_settings_of_an_item_are_edited_beside_the_bar_and_leave_the_layout_alone(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx);
    fixture.click("layout-item:tools.clock", cx);
    fixture.click(SECONDS, cx);

    let edits = fixture.tools.edits();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].0, "seconds");
    assert_eq!(edits[0].1, FieldEdit::Set(SettingValue::Bool(true)));
    assert!(
        fixture.stored(cx).presets.is_empty(),
        "the layout is untouched"
    );

    fixture.click("layout-undo", cx);
    assert_eq!(
        fixture.tools.edits().len(),
        1,
        "undo has nothing of the form to take back"
    );
    cx.update(|cx| {
        assert_eq!(
            fixture.tools.values(cx).get("seconds"),
            Some(&SettingValue::Bool(true))
        );
    });
}

#[gpui_kit::test]
fn an_item_of_a_disabled_plugin_says_so_and_offers_to_enable_it(cx: &mut TestAppContext) {
    let fixture = open(cx);
    let opened = opened_plugins(cx);
    fixture.click("layout-item:off.item", cx);

    assert_eq!(
        fixture.label("layout-owner", cx).as_deref(),
        Some("Disabled \u{b7} Part of Switched off")
    );
    fixture.click("layout-enable-plugin", cx);
    assert_eq!(*opened.borrow(), [Some("off".to_owned())]);
}

#[gpui_kit::test]
fn the_settings_are_editable_on_a_built_in_layout_that_is_not_in_use(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-choice:swapped", cx);
    fixture.click("layout-item:tools.clock", cx);

    assert!(fixture.has("layout-read-only", cx));
    assert!(
        !fixture.has("layout-controls", cx),
        "the arrangement is read-only"
    );
    fixture.click(SECONDS, cx);
    assert_eq!(fixture.tools.edits().len(), 1);
}

#[gpui_kit::test]
fn a_panel_group_has_an_arrangement_and_a_side(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-item:group:navigation", cx);

    assert!(fixture.has("layout-inspector:group:navigation", cx));
    assert!(fixture.has("layout-side", cx));
}

#[gpui_kit::test]
fn an_item_the_plugin_declares_no_settings_for_shows_none(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-item:tools.cpu", cx);

    assert!(fixture.has("layout-no-settings", cx));
    assert!(!fixture.has(SECONDS, cx));
}

#[gpui_kit::test]
fn clicking_a_panel_in_the_preview_selects_the_panel_and_not_its_group(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-preview:shell.files", cx);

    assert!(fixture.has("layout-inspector:shell.files", cx));
    assert!(!fixture.has("layout-inspector:group:navigation", cx));
}

#[test]
fn a_group_that_follows_its_zone_has_no_side_of_its_own() {
    let group = |id: &'static str, side| BarItem::Group {
        id: GroupId::new(id),
        side,
        primary: false,
        members: Vec::new(),
    };
    let preset = PresetId::from_owned("p1".to_owned());
    let settings = LayoutSettings {
        active: Some(preset.clone()),
        presets: vec![LayoutPreset {
            id: preset,
            name: "Work".to_owned(),
            bar: Bar {
                center: vec![
                    group("navigation", Some(Side::Right)),
                    group("assistant", None),
                ],
                ..Bar::default()
            },
            ..LayoutPreset::default()
        }],
    };

    let snapshot = Snapshot::new(
        &settings,
        &PluginEnablement::default(),
        false,
        None,
        &catalog(),
    );

    assert_eq!(
        snapshot.side_override(&GroupId::new("navigation")),
        Some(Side::Right)
    );
    assert_eq!(snapshot.side_override(&GroupId::new("assistant")), None);
}

mod user_plugins {
    use std::{borrow::Cow, rc::Rc, sync::Arc};

    use gpui_kit::TestAppContext;
    use nocterm_plugin_model::{
        ContributionId, ContributionKind, FieldGroup, FieldKind, FieldSpec, Icon, PluginId,
        PluginSet, SettingsSchema, UserContribution, UserEntry, UserPlugin, Zone,
    };
    use nocterm_settings::SettingsDocument;
    use nocterm_ui::{FakePluginSettings, PluginSettingsSource, SchemaRef, UserPluginSettings};
    use nocterm_window_layout::{Bar, BarItem, LayoutPreset, LayoutSettings, PresetId};

    use super::opened_plugins;
    use crate::{
        UserFacts, UserPluginFacts,
        tests::{Fixture, Plugins, catalog, open_plugins},
    };

    const ID: &str = "user-acme-clock";
    const FACE: &str = "user-acme-clock.face";

    static SCHEMA: SettingsSchema = SettingsSchema {
        version: 1,
        groups: Cow::Borrowed(&[FieldGroup {
            title: Cow::Borrowed("Clock"),
            contribution: Some(ContributionId::new(FACE)),
            fields: Cow::Borrowed(&[FieldSpec {
                renamed_from: None,
                key: Cow::Borrowed("seconds"),
                label: Cow::Borrowed("Show seconds"),
                help: None,
                kind: FieldKind::Toggle { default: false },
            }]),
        }]),
    };

    struct Settings(FakePluginSettings);

    impl UserPluginSettings for Settings {
        fn find(
            &self,
            plugin: &PluginId,
            _: &gpui_kit::App,
        ) -> Option<(SchemaRef, Rc<dyn PluginSettingsSource>)> {
            (plugin.as_str() == ID).then(|| {
                (
                    SchemaRef::User(Arc::new(SCHEMA.clone())),
                    Rc::new(self.0.clone()) as _,
                )
            })
        }
    }

    struct Facts;

    impl UserPluginFacts for Facts {
        fn facts(&self, plugin: &PluginId, _: &gpui_kit::App) -> Option<UserFacts> {
            (plugin.as_str() == ID).then(|| UserFacts {
                publisher: "acme".to_owned(),
                fingerprint: "0123456789abcdef".to_owned(),
                capabilities: vec!["See the current time.".to_owned()],
            })
        }
    }

    fn user_set() -> Arc<PluginSet> {
        let face = UserContribution::clamp(
            ContributionId::owned(FACE),
            ContributionKind::Widget,
            "Face",
            Icon::Clock,
            Zone::Right,
            10,
        )
        .unwrap();
        let plugin = UserPlugin::new(
            PluginId::owned(ID),
            "Acme Clock",
            "A clock",
            vec![face],
            SCHEMA.clone(),
        )
        .unwrap();
        let entry = UserEntry {
            plugin,
            enabled: true,
            placeable: true,
        };
        Arc::new(catalog().with_users(vec![entry], 1).unwrap())
    }

    fn open_user(facts: bool, cx: &mut TestAppContext) -> (Fixture, FakePluginSettings) {
        let preset = PresetId::from_owned("p1".to_owned());
        let mut document = SettingsDocument::default();
        document.register::<LayoutSettings>();
        document.set(LayoutSettings {
            active: Some(preset.clone()),
            presets: vec![LayoutPreset {
                id: preset,
                name: "Work".to_owned(),
                bar: Bar {
                    right: vec![BarItem::Item {
                        id: ContributionId::owned(FACE),
                    }],
                    ..Bar::default()
                },
                ..LayoutPreset::default()
            }],
        });
        let config = nocterm_store::SettingsConfig::in_memory(document);
        let source = cx.update(|cx| FakePluginSettings::new(&SCHEMA, cx));
        let plugins = Plugins {
            set: user_set(),
            user: Some(Rc::new(Settings(source.clone()))),
            facts: facts.then(|| Rc::new(Facts) as Rc<dyn UserPluginFacts>),
        };
        (open_plugins(config, plugins, cx), source)
    }

    #[gpui_kit::test]
    fn an_item_of_a_user_plugin_shows_what_the_host_knows_of_it(cx: &mut TestAppContext) {
        let (fixture, _) = open_user(true, cx);
        fixture.click("layout-item:user-acme-clock.face", cx);

        let label = |id, cx: &mut TestAppContext| fixture.label(id, cx);
        assert_eq!(
            label("layout-user-name", cx).as_deref(),
            Some("Plugin \u{b7} Acme Clock")
        );
        assert_eq!(
            label("layout-user-id", cx).as_deref(),
            Some("Id: user-acme-clock")
        );
        assert_eq!(
            label("layout-user-publisher", cx).as_deref(),
            Some("Publisher: acme (unverified)")
        );
        assert_eq!(
            label("layout-user-fingerprint", cx).as_deref(),
            Some("Fingerprint: 0123456789abcdef")
        );
        assert_eq!(
            label("layout-user-capabilities", cx).as_deref(),
            Some("Allowed: See the current time.")
        );
    }

    #[gpui_kit::test]
    fn the_settings_of_a_user_item_are_framed_as_the_plugins_and_edit_its_source(
        cx: &mut TestAppContext,
    ) {
        let (fixture, source) = open_user(true, cx);
        fixture.click("layout-item:user-acme-clock.face", cx);

        assert_eq!(
            fixture
                .label("plugin-settings-frame:user-acme-clock", cx)
                .as_deref(),
            Some("Plugin \u{b7} Acme Clock")
        );
        fixture.click("plugin-setting:user-acme-clock:seconds:", cx);
        assert_eq!(source.edits().len(), 1);
    }

    #[gpui_kit::test]
    fn a_user_item_links_to_its_plugin_in_the_plugins_page(cx: &mut TestAppContext) {
        let (fixture, _) = open_user(true, cx);
        let opened = opened_plugins(cx);
        fixture.click("layout-item:user-acme-clock.face", cx);

        fixture.click("layout-manage-plugin", cx);

        assert_eq!(*opened.borrow(), [Some(ID.to_owned())]);
    }

    #[gpui_kit::test]
    fn without_the_hosts_facts_a_user_item_shows_its_name_and_id_only(cx: &mut TestAppContext) {
        let (fixture, _) = open_user(false, cx);
        fixture.click("layout-item:user-acme-clock.face", cx);

        assert!(fixture.has("layout-user-id", cx));
        assert!(!fixture.has("layout-user-publisher", cx));
    }
}

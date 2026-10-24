use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, GroupId, GroupSpec, Icon, Placement,
    PluginCatalog, SettingsSchema, SystemContribution, SystemPlugin, Zone,
};

use super::*;

const fn widget(id: &'static str, order: u16) -> SystemContribution {
    SystemContribution {
        id: ContributionId::new(id),
        kind: ContributionKind::Widget,
        title: "Widget",
        icon: Icon::Clock,
        placement: Placement {
            zone: Zone::Right,
            order,
            group: None,
            auto: AutoPlace::Show,
        },
        panel: None,
        preview: None,
        pinned: false,
    }
}

static OPTIONAL: SystemPlugin = SystemPlugin {
    id: PluginId::new("optional"),
    name: "Optional",
    description: "A plugin that can be switched off",
    required: false,
    contributions: &[widget("optional.clock", 10)],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: None,
};
static REQUIRED: SystemPlugin = SystemPlugin {
    id: PluginId::new("required"),
    name: "Required",
    description: "A plugin that cannot be switched off",
    required: true,
    contributions: &[widget("required.clock", 20)],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: None,
};
const CORE: &[SystemContribution] = &[widget("shell.notice", 5)];

fn catalog() -> PluginCatalog {
    let groups = [GroupSpec {
        id: GroupId::new("navigation"),
        title: "Sidebar",
        primary: true,
    }];
    PluginCatalog::new(&groups, &[CORE], &[&OPTIONAL, &REQUIRED]).unwrap()
}

fn ids(names: &[&'static str]) -> Vec<PluginId> {
    names.iter().map(|name| PluginId::new(name)).collect()
}

fn read(text: &str) -> PluginEnablement {
    let mut section: PluginEnablement = toml::from_str(text).unwrap();
    section.sanitize();
    section
}

#[test]
fn nothing_is_disabled_by_default() {
    assert!(PluginEnablement::default().disabled.is_empty());
    assert!(
        PluginEnablement::default()
            .effective_disabled(&catalog())
            .is_empty()
    );
}

#[test]
fn sanitizing_drops_invalid_ids_and_repeats() {
    let section = read(r#"disabled = ["optional", "Not Valid", "", "optional", "other"]"#);
    assert_eq!(section.disabled, ids(&["optional", "other"]));
}

#[test]
fn sanitizing_caps_the_list() {
    let names: Vec<String> = (0..MAX_DISABLED + 10).map(|n| format!("p{n}")).collect();
    let section = read(&format!("disabled = {names:?}"));
    assert_eq!(section.disabled.len(), MAX_DISABLED);
    assert_eq!(section.disabled[0].as_str(), "p0");
}

#[test]
fn an_id_missing_from_this_build_is_kept() {
    let section = read(r#"disabled = ["projects"]"#);
    assert_eq!(section.disabled, ids(&["projects"]));
}

#[test]
fn the_effective_set_ignores_required_unknown_and_core_ids() {
    let section = read(r#"disabled = ["optional", "required", "projects", "shell"]"#);
    let effective = section.effective_disabled(&catalog());
    assert_eq!(
        effective.into_iter().collect::<Vec<_>>(),
        ids(&["optional"])
    );
}

#[test]
fn switching_a_plugin_off_and_on_edits_the_list_once() {
    let mut section = PluginEnablement::default();
    let id = PluginId::new("optional");
    section.set_enabled(&id, false);
    section.set_enabled(&id, false);
    assert_eq!(section.disabled, ids(&["optional"]));
    section.set_enabled(&id, true);
    assert!(section.disabled.is_empty());
}

#[test]
fn the_section_is_stored_under_plugins() {
    assert_eq!(PluginEnablement::KEY, "plugins");
}

#[test]
fn a_table_that_does_not_read_is_reported_and_a_good_one_is_not() {
    let document = |text: &str| {
        let mut document = SettingsDocument::from_table(text.parse().unwrap());
        document.register::<PluginEnablement>();
        document
    };
    for broken in [
        "[plugins]\ndisabled = \"monitor\"\n",
        "[plugins]\ndisabled = [\"monitor\", 1]\n",
    ] {
        assert!(
            PluginEnablement::read_error(&document(broken)).is_some(),
            "{broken}"
        );
    }
    assert_eq!(
        PluginEnablement::read_error(&document("[plugins]\ndisabled = [\"monitor\"]\n")),
        None
    );
    assert_eq!(PluginEnablement::read_error(&document("")), None);
}

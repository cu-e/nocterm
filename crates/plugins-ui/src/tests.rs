//! The page lists the plugins, switches them through its port and edits their settings.

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::BTreeSet,
    rc::Rc,
    sync::Arc,
};

use gpui_kit::{
    App, Entity, Subscription, Task, TestAppContext, WindowOptions, test::TestWindowExt as _,
};
use nocterm_core::SharedCause;
use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, FieldEdit, FieldGroup, FieldKind, FieldSpec,
    GroupId, GroupSpec, Icon, Placement, PluginCatalog, PluginId, SettingValue, SettingsSchema,
    SystemContribution, SystemPlugin, Zone,
};
use nocterm_ui::{FakePluginSettings, PluginSettingsDirectory, SettingsPageHandle};
use nocterm_workspace::{OpenLayoutSettings, OpenPluginPage};

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

const GROUP: FieldGroup = FieldGroup {
    title: Cow::Borrowed("Display"),
    contribution: None,
    fields: Cow::Borrowed(&[FieldSpec {
        key: Cow::Borrowed("verbose"),
        label: Cow::Borrowed("Verbose"),
        help: None,
        kind: FieldKind::Toggle { default: false },
    }]),
};

static TOOLS: SystemPlugin = SystemPlugin {
    id: PluginId::new("tools"),
    name: "Tools",
    description: "Small helpers",
    required: false,
    contributions: &[widget("tools.clock", 10), widget("tools.cpu", 20)],
    settings_section: Some("tools"),
    settings: SettingsSchema {
        groups: Cow::Borrowed(&[GROUP]),
    },
    settings_page: Some("tools"),
};
static AGENT: SystemPlugin = SystemPlugin {
    id: PluginId::new("agent"),
    name: "AI Agents",
    description: "Chats with AI agents",
    required: true,
    contributions: &[widget("agent.chats", 30)],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: Some("ai"),
};
static PLAIN: SystemPlugin = SystemPlugin {
    id: PluginId::new("plain"),
    name: "Plain",
    description: "Nothing to set",
    required: false,
    contributions: &[widget("plain.note", 40)],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: None,
};
const CORE: &[SystemContribution] = &[widget("shell.notice", 5)];

fn catalog() -> Arc<PluginCatalog> {
    let groups = [GroupSpec {
        id: GroupId::new("navigation"),
        title: "Sidebar",
        primary: true,
    }];
    Arc::new(PluginCatalog::new(&groups, &[CORE], &[&TOOLS, &AGENT, &PLAIN]).unwrap())
}

/// A `PluginToggles` that remembers the switches and can refuse the next one.
#[derive(Clone)]
struct FakeToggles {
    disabled: Rc<RefCell<BTreeSet<PluginId>>>,
    calls: Rc<RefCell<Vec<(PluginId, bool)>>>,
    refuse: Rc<Cell<bool>>,
    changed: Entity<()>,
}

impl FakeToggles {
    fn new(cx: &mut App) -> Self {
        Self {
            disabled: Rc::default(),
            calls: Rc::default(),
            refuse: Rc::default(),
            changed: cx.new(|_| ()),
        }
    }
    fn calls(&self) -> Vec<(PluginId, bool)> {
        self.calls.borrow().clone()
    }
}

impl PluginToggles for FakeToggles {
    fn enabled(&self, plugin: &PluginId, _: &App) -> bool {
        !self.disabled.borrow().contains(plugin)
    }
    fn set_enabled(
        &self,
        plugin: &PluginId,
        enabled: bool,
        cx: &mut App,
    ) -> Task<Result<(), EnablementError>> {
        self.calls.borrow_mut().push((plugin.clone(), enabled));
        if self.refuse.take() {
            let cause = std::io::Error::other("disk full");
            return Task::ready(Err(EnablementError::Write(SharedCause::new(cause))));
        }
        if *plugin == AGENT.id && !enabled {
            return Task::ready(Err(EnablementError::Required(plugin.clone())));
        }
        if enabled {
            self.disabled.borrow_mut().remove(plugin);
        } else {
            self.disabled.borrow_mut().insert(plugin.clone());
        }
        self.changed.update(cx, |_, cx| cx.notify());
        Task::ready(Ok(()))
    }
    fn observe(&self, cx: &mut App, mut on_change: Box<dyn FnMut(&mut App)>) -> Subscription {
        cx.observe(&self.changed, move |_, cx| on_change(cx))
    }
}

struct Fixture {
    window: gpui_kit::AnyWindowHandle,
    page: Entity<PluginsPage>,
    toggles: FakeToggles,
    tools: FakePluginSettings,
}

fn open(cx: &mut TestAppContext) -> Fixture {
    cx.update(|cx| {
        gpui_kit::init(cx);
        let store = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        nocterm_ui::init(nocterm_ui::DesignTokens::builtin(), store, cx);
        let toggles = FakeToggles::new(cx);
        let tools = FakePluginSettings::new(&TOOLS.settings, cx);
        let directory = PluginSettingsDirectory::new().with_system(&TOOLS, Rc::new(tools.clone()));
        let (window, page) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                PluginsPage::new(catalog(), Rc::new(toggles.clone()), directory, window, cx)
            })
        })
        .unwrap();
        Fixture {
            window,
            page,
            toggles,
            tools,
        }
    })
}

fn frame(
    fixture: &Fixture,
    cx: &mut TestAppContext,
    check: impl FnOnce(&mut gpui_kit::Window, &mut App),
) {
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        check(window, cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_list_shows_the_plugins_in_catalog_order_with_their_state(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture
        .toggles
        .disabled
        .borrow_mut()
        .insert(PluginId::new("plain"));
    frame(&fixture, cx, |window, _| {
        let rows: Vec<_> = ["tools", "agent", "plain"]
            .map(|id| window.find(format!("plugin-row:{id}")).bounds().origin.y)
            .into();
        assert!(rows[0] < rows[1] && rows[1] < rows[2], "catalog order");
        assert!(
            window.try_find("plugin-row:shell").is_none(),
            "no core entries"
        );
        assert!(window.try_find("plugin-chip:agent:Required").is_some());
        assert!(window.try_find("plugin-chip:plain:Disabled").is_some());
        assert!(window.try_find("plugin-chip:tools:Disabled").is_none());
        assert!(window.try_find("plugin-chip:tools:Required").is_none());
    });
}

#[gpui_kit::test]
fn a_required_plugin_has_a_switch_that_cannot_be_used(cx: &mut TestAppContext) {
    let fixture = open(cx);
    frame(&fixture, cx, |window, _| {
        assert_eq!(window.find("plugin-detail").label(), Some("Tools"));
    });
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-row:agent", cx)
    });
    frame(&fixture, cx, |window, _| {
        assert_eq!(window.find("plugin-detail").label(), Some("AI Agents"));
        assert_eq!(window.find("plugin-enabled").checked(), Some(true));
    });
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-enabled", cx)
    });
    cx.run_until_parked();
    assert!(fixture.toggles.calls().is_empty());
}

#[gpui_kit::test]
fn the_switch_turns_a_plugin_off_and_on_through_the_port(cx: &mut TestAppContext) {
    let fixture = open(cx);
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-enabled", cx)
    });
    cx.run_until_parked();
    assert_eq!(fixture.toggles.calls(), [(PluginId::new("tools"), false)]);
    frame(&fixture, cx, |window, _| {
        assert_eq!(window.find("plugin-enabled").checked(), Some(false));
        assert!(window.try_find("plugin-chip:tools:Disabled").is_some());
    });
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-enabled", cx)
    });
    cx.run_until_parked();
    assert_eq!(
        fixture.toggles.calls().last(),
        Some(&(PluginId::new("tools"), true))
    );
    frame(&fixture, cx, |window, _| {
        assert_eq!(window.find("plugin-enabled").checked(), Some(true));
        assert!(window.try_find("plugin-chip:tools:Disabled").is_none());
    });
}

#[gpui_kit::test]
fn a_refused_switch_says_why_and_leaves_the_plugin_as_it_was(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.toggles.refuse.set(true);
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-enabled", cx)
    });
    cx.run_until_parked();
    frame(&fixture, cx, |window, _| {
        let refusal = window.find("plugin-refusal");
        assert!(refusal.label().unwrap().contains("Could not save"));
        assert_eq!(window.find("plugin-enabled").checked(), Some(true));
    });
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-row:plain", cx)
    });
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("plugin-refusal").is_none());
    });
}

#[gpui_kit::test]
fn the_selected_plugin_shows_its_form_and_writes_through_its_source(cx: &mut TestAppContext) {
    let fixture = open(cx);
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-setting:tools:verbose:", cx);
    });
    cx.run_until_parked();
    assert_eq!(
        fixture.tools.edits(),
        [(
            "verbose".to_owned(),
            FieldEdit::Set(SettingValue::Bool(true))
        )]
    );
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-row:plain", cx)
    });
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("plugin-setting:tools:verbose:").is_none());
        assert!(window.try_find("plugin-detail").is_some());
    });
}

#[gpui_kit::test]
fn revealing_a_plugin_id_selects_that_plugin(cx: &mut TestAppContext) {
    let fixture = open(cx);
    cx.update_window(fixture.window, |_, window, cx| {
        assert!(fixture.page.reveal("plain", window, cx));
        assert!(!fixture.page.reveal("missing", window, cx));
        window.render_frame(cx);
        assert_eq!(window.find("plugin-detail").label(), Some("Plain"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_plugin_with_its_own_page_links_to_it_by_its_id(cx: &mut TestAppContext) {
    let fixture = open(cx);
    let opened = Rc::new(RefCell::new(Vec::new()));
    cx.update(|cx| {
        let opened = opened.clone();
        cx.on_action(move |action: &OpenPluginPage, _| {
            opened.borrow_mut().push(action.plugin.to_string());
        });
    });
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-page-link", cx);
    });
    assert_eq!(*opened.borrow(), ["tools"]);
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-row:agent", cx)
    });
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-page-link", cx);
    });
    assert_eq!(
        *opened.borrow(),
        ["tools", "agent"],
        "any plugin with a page"
    );
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-row:plain", cx)
    });
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("plugin-page-link").is_none());
    });
}

#[gpui_kit::test]
fn the_detail_lists_what_the_plugin_contributes(cx: &mut TestAppContext) {
    let fixture = open(cx);
    frame(&fixture, cx, |window, _| {
        for id in ["tools.clock", "tools.cpu"] {
            let row = window.find(format!("plugin-contribution:{id}"));
            assert_eq!(row.label(), Some("Widget"));
        }
        assert!(window.try_find("plugin-contribution:plain.note").is_none());
    });
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-row:plain", cx)
    });
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("plugin-contribution:plain.note").is_some());
        assert!(window.try_find("plugin-contribution:tools.clock").is_none());
    });
}

#[gpui_kit::test]
fn a_plugin_without_settings_says_so(cx: &mut TestAppContext) {
    let fixture = open(cx);
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-row:plain", cx)
    });
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("plugin-page-link").is_none());
        assert!(window.try_find("plugin-setting:plain:verbose:").is_none());
    });
}

#[gpui_kit::test]
fn a_plugin_links_to_its_first_contribution_in_the_layout_editor(cx: &mut TestAppContext) {
    let fixture = open(cx);
    let selected = Rc::new(RefCell::new(Vec::new()));
    cx.update(|cx| {
        let selected = selected.clone();
        cx.on_action(move |action: &OpenLayoutSettings, _| {
            selected.borrow_mut().push(action.select.clone());
        });
    });
    frame(&fixture, cx, |window, cx| {
        window.click("plugin-layout-link", cx);
    });
    assert_eq!(*selected.borrow(), [Some("tools.clock".into())]);
}

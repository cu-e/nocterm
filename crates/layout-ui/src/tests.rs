//! The editor edits the layouts through the store: drags, menus and keys in the bar preview,
//! presets and history.

use std::{borrow::Cow, rc::Rc, sync::Arc};

use gpui_kit::{
    AnyWindowHandle, AppContext as _, TestAppContext, WindowOptions, test::TestWindowExt as _,
};
use nocterm_plugin_enablement::PluginEnablement;
use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, FieldGroup, FieldKind, FieldSpec, GroupId,
    GroupSpec, Icon, PanelChrome, PanelTraits, Placement, PluginCatalog, PluginId, PluginSet,
    SettingsSchema, SystemContribution, SystemPlugin, WidthClass, Zone,
};
use nocterm_settings::SettingsDocument;
use nocterm_store::SettingsHandle;
use nocterm_ui::StaticPluginSource;
use nocterm_window_layout::LayoutSettings;

use crate::section::LayoutEditorSection;

mod dnd;
mod editing;
mod geometry;
mod grouping;
mod history;
mod menu;
mod presets;

const fn widget(
    id: &'static str,
    title: &'static str,
    zone: Zone,
    order: u16,
) -> SystemContribution {
    SystemContribution {
        id: ContributionId::new(id),
        kind: ContributionKind::Widget,
        title,
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

const fn panel(
    id: &'static str,
    title: &'static str,
    group: &'static str,
    order: u16,
) -> SystemContribution {
    let mut panel = widget(id, title, Zone::Left, order);
    panel.kind = ContributionKind::Panel;
    panel.placement.group = Some(group);
    panel.panel = Some(PanelTraits {
        width: WidthClass::Navigation,
        chrome: PanelChrome::Titled,
    });
    panel
}

const fn pinned(mut widget: SystemContribution) -> SystemContribution {
    widget.pinned = true;
    widget
}

const CORE: &[SystemContribution] = &[
    panel("shell.servers", "Servers", "navigation", 10),
    panel("shell.files", "Files", "navigation", 20),
    panel("shell.agent", "Agent", "assistant", 40),
    pinned(widget("shell.notices", "Notices", Zone::Right, 10)),
    widget("shell.transfers", "Transfers", Zone::Right, 20),
    widget("shell.terminal", "Terminal", Zone::Right, 30),
];

const CLOCK_GROUP: FieldGroup = FieldGroup {
    title: Cow::Borrowed("Clock"),
    contribution: Some(ContributionId::new("tools.clock")),
    fields: Cow::Borrowed(&[FieldSpec {
        renamed_from: None,
        key: Cow::Borrowed("seconds"),
        label: Cow::Borrowed("Show seconds"),
        help: None,
        kind: FieldKind::Toggle { default: false },
    }]),
};

pub(super) static TOOLS: SystemPlugin = SystemPlugin {
    id: PluginId::new("tools"),
    name: "Tools",
    description: "Small helpers",
    required: false,
    contributions: &[
        widget("tools.clock", "Clock", Zone::Right, 25),
        widget("tools.cpu", "Processor", Zone::Right, 26),
    ],
    settings_section: Some("tools"),
    settings: SettingsSchema {
        version: 1,
        groups: Cow::Borrowed(&[CLOCK_GROUP]),
    },
    settings_page: Some("tools"),
};

static PLAIN: SystemPlugin = SystemPlugin {
    id: PluginId::new("plain"),
    name: "Plain",
    description: "Nothing to set",
    required: false,
    contributions: &[widget("plain.note", "Note", Zone::Right, 40)],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: None,
};

static OFF: SystemPlugin = SystemPlugin {
    id: PluginId::new("off"),
    name: "Switched off",
    description: "Turned off by the user",
    required: false,
    contributions: &[widget("off.item", "Dormant", Zone::Right, 50)],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: None,
};

fn plugin_catalog() -> PluginCatalog {
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
    PluginCatalog::new(&groups, &[CORE], &[&TOOLS, &PLAIN, &OFF]).unwrap()
}

fn catalog() -> Arc<PluginSet> {
    Arc::new(PluginSet::from(plugin_catalog()))
}

pub(super) struct Fixture {
    pub(super) window: AnyWindowHandle,
    pub(super) settings: SettingsHandle,
}

pub(super) fn open(cx: &mut TestAppContext) -> Fixture {
    open_with(LayoutSettings::default(), cx)
}

/// An editor over the stored layouts `stored`, with the plugin `off` disabled.
pub(super) fn open_with(stored: LayoutSettings, cx: &mut TestAppContext) -> Fixture {
    let mut document = SettingsDocument::default();
    document.register::<LayoutSettings>();
    document.set(stored);
    open_over(document, cx)
}

/// An editor over the settings `document`, with the plugin `off` disabled.
pub(super) fn open_over(document: SettingsDocument, cx: &mut TestAppContext) -> Fixture {
    open_config(
        nocterm_store::SettingsConfig::in_memory(prepared(document)),
        cx,
    )
}

/// An editor over the default layouts kept in a settings file in `directory`, which is written
/// and waited for like the application's own.
pub(super) fn open_on_disk(directory: &std::path::Path, cx: &mut TestAppContext) -> Fixture {
    let file = nocterm_settings::SettingsFile::new(directory.join("settings.toml"));
    let document = prepared(SettingsDocument::default());
    open_config(nocterm_store::SettingsConfig::new(document, file), cx)
}

/// The `document` with the sections the editor reads and the plugin `off` disabled.
fn prepared(mut document: SettingsDocument) -> SettingsDocument {
    document.register::<PluginEnablement>();
    document.update::<PluginEnablement>(|enablement| {
        enablement.set_enabled(&plugin_catalog(), &PluginId::new("off"), false);
    });
    document
}

fn open_config(config: nocterm_store::SettingsConfig, cx: &mut TestAppContext) -> Fixture {
    let (window, settings) = cx.update(|cx| {
        gpui_kit::init(cx);
        let settings = config.install(cx);
        nocterm_ui::init(nocterm_ui::DesignTokens::builtin(), settings.clone(), cx);
        settings.register::<LayoutSettings>(cx);
        settings.register::<PluginEnablement>(cx);
        let (window, _) = gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|cx| {
                LayoutEditorSection::new(
                    settings.clone(),
                    Rc::new(StaticPluginSource::new(catalog())),
                    None,
                    false,
                    cx,
                )
            })
        })
        .unwrap();
        (window, settings)
    });
    cx.update_window(window, |_, window, _| window.activate_window())
        .unwrap();
    cx.simulate_window_resize(
        window,
        gpui_kit::size(gpui_kit::px(1200.), gpui_kit::px(2400.)),
    );
    cx.run_until_parked();
    Fixture { window, settings }
}

impl Fixture {
    /// Draws a frame and lets `check` look at the window.
    pub(super) fn frame(
        &self,
        cx: &mut TestAppContext,
        check: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
    ) {
        // The first frame records the width of the bar preview, which the second one is drawn for.
        cx.update_window(self.window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            check(window, cx);
        })
        .unwrap();
    }

    /// Drags the element `from` onto the element `to` and lets the writes it caused settle.
    pub(super) fn drag(&self, from: &str, to: &str, cx: &mut TestAppContext) {
        let (from, to) = (from.to_owned(), to.to_owned());
        self.frame(cx, |window, cx| window.drag_to(from, to, cx));
        cx.run_until_parked();
    }

    /// Clicks the element `id` in the frame drawn now and lets the writes it caused settle.
    pub(super) fn click(&self, id: &str, cx: &mut TestAppContext) {
        let id = id.to_owned();
        self.frame(cx, |window, cx| window.click(id, cx));
        cx.run_until_parked();
    }

    pub(super) fn stored(&self, cx: &mut TestAppContext) -> LayoutSettings {
        cx.update(|cx| self.settings.setting::<LayoutSettings>(cx).clone())
    }

    /// Runs the executors until the stored layouts satisfy `done`, for a store whose writes
    /// take real time. Panics when they never do.
    pub(super) fn wait_for(
        &self,
        cx: &mut TestAppContext,
        done: impl Fn(&LayoutSettings) -> bool,
    ) -> LayoutSettings {
        cx.executor().allow_parking();
        for _ in 0..400 {
            cx.run_until_parked();
            let stored = self.stored(cx);
            if done(&stored) {
                return stored;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the layouts never reached the state waited for");
    }

    /// The accessible label of the element `id` in the frame drawn now.
    pub(super) fn label(&self, id: &str, cx: &mut TestAppContext) -> Option<String> {
        let mut found = None;
        self.frame(cx, |window, _| {
            found = window
                .try_find(id.to_owned())
                .and_then(|element| element.label().map(str::to_owned));
        });
        found
    }

    /// Resizes the window to `width` pixels, as a Settings page of that width would be.
    pub(super) fn resize(&self, width: f32, cx: &mut TestAppContext) {
        cx.simulate_window_resize(
            self.window,
            gpui_kit::size(gpui_kit::px(width), gpui_kit::px(2400.)),
        );
        cx.run_until_parked();
    }

    pub(super) fn keys(&self, keys: &str, cx: &mut TestAppContext) {
        cx.simulate_keystrokes(self.window, keys);
        cx.run_until_parked();
    }

    /// Whether an element with `id` is in the frame drawn now.
    pub(super) fn has(&self, id: &str, cx: &mut TestAppContext) -> bool {
        let mut found = false;
        self.frame(cx, |window, _| {
            found = window.try_find(id.to_owned()).is_some()
        });
        found
    }

    /// The items among `ids` that the bar preview shows, from left to right. An id `group:g`
    /// names the group `g`.
    pub(super) fn left_to_right(&self, ids: &[&str], cx: &mut TestAppContext) -> Vec<String> {
        let mut found = Vec::new();
        self.frame(cx, |window, _| {
            for id in ids {
                if let Some(element) = window.try_find(preview_id(id)) {
                    found.push((f32::from(element.bounds().origin.x), (*id).to_owned()));
                }
            }
        });
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found.into_iter().map(|(_, id)| id).collect()
    }

    /// Whether the preview shows the item `id` (or `group:g`) selected.
    pub(super) fn is_selected(&self, id: &str, cx: &mut TestAppContext) -> bool {
        let mut selected = false;
        self.frame(cx, |window, _| {
            selected = window
                .try_find(preview_id(id))
                .and_then(|element| element.selected())
                .unwrap_or(false);
        });
        selected
    }

    /// Opens the menu of the element `id` and chooses the entry `label` on its first level,
    /// then lets the writes it caused settle.
    pub(super) fn choose(&self, id: &str, label: &str, cx: &mut TestAppContext) {
        let id = id.to_owned();
        self.frame(cx, |window, cx| window.right_click(id, cx));
        cx.run_until_parked();
        let label = label.to_owned();
        self.frame(cx, |window, cx| {
            let mut menu = window.within("popup-menu");
            let at = (0usize..64)
                .find(|at| {
                    menu.try_find(*at)
                        .is_some_and(|entry| entry.label() == Some(label.as_str()))
                })
                .unwrap_or_else(|| panic!("the menu has no entry \u{201c}{label}\u{201d}"));
            menu.click(at, cx);
        });
        cx.run_until_parked();
    }
}

/// The element of the bar preview that draws `id`, or the group `g` for `group:g`.
pub(super) fn preview_id(id: &str) -> String {
    match id.strip_prefix("group:") {
        Some(group) => format!("layout-preview-group:{group}"),
        None => format!("layout-preview:{id}"),
    }
}

pub(super) const RIGHT: [&str; 4] = [
    "shell.notices",
    "shell.transfers",
    "tools.clock",
    "shell.terminal",
];

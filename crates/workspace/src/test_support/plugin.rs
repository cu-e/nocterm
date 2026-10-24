//! A plugin catalog for tests of panel and widget registration.
//!
//! [`TEST_PLUGIN`] offers two sidebar panels, one expandable side panel, one expandable
//! sidebar panel and one status bar widget; [`deps_for`] and [`deps_for_core`] compose them, or any real plugin, with the
//! shell's own contributions into the strict dependencies a window needs.
use crate::{Deps, plugin};
use gpui_kit::App;
use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, PanelChrome, PanelTraits, Placement, PluginId,
    SystemContribution, SystemPlugin, WidthClass, Zone,
};

const fn panel(
    id: &'static str,
    title: &'static str,
    order: u16,
    zone: Zone,
    group: &'static str,
    traits: PanelTraits,
) -> SystemContribution {
    SystemContribution {
        id: ContributionId::new(id),
        kind: ContributionKind::Panel,
        title,
        icon: nocterm_plugin_model::Icon::Layers,
        placement: Placement {
            zone,
            order,
            group: Some(group),
            auto: AutoPlace::Show,
        },
        panel: Some(traits),
        preview: None,
        pinned: false,
    }
}

const TITLED: PanelTraits = PanelTraits {
    width: WidthClass::Navigation,
    chrome: PanelChrome::Titled,
};
const OWN_HEADER: PanelTraits = PanelTraits {
    width: WidthClass::Assistant,
    chrome: PanelChrome::OwnHeader,
};

/// A plugin with the ids `test.panel-a`, `test.panel-b`, `test.expandable`, `test.widget` and
/// `test.sidebar-expandable`.
pub static TEST_PLUGIN: SystemPlugin = SystemPlugin {
    id: PluginId::new("test"),
    name: "Test",
    description: "Panels and a widget for registration tests",
    required: false,
    contributions: &[
        panel(
            "test.panel-a",
            "Panel A",
            10,
            Zone::Left,
            "navigation",
            TITLED,
        ),
        panel(
            "test.panel-b",
            "Panel B",
            20,
            Zone::Left,
            "navigation",
            TITLED,
        ),
        panel(
            "test.expandable",
            "Expandable",
            40,
            Zone::Right,
            "assistant",
            OWN_HEADER,
        ),
        SystemContribution {
            id: ContributionId::new("test.widget"),
            kind: ContributionKind::Widget,
            title: "Widget",
            icon: nocterm_plugin_model::Icon::Clock,
            placement: Placement {
                zone: Zone::Left,
                order: 60,
                group: None,
                auto: AutoPlace::Show,
            },
            panel: None,
            preview: None,
            pinned: false,
        },
        panel(
            "test.sidebar-expandable",
            "Sidebar expandable",
            70,
            Zone::Left,
            "navigation",
            OWN_HEADER,
        ),
    ],
};

/// A required plugin with one expandable panel, `test-required.expandable`.
pub static REQUIRED_PLUGIN: SystemPlugin = SystemPlugin {
    id: PluginId::new("test-required"),
    name: "Required",
    description: "A plugin that cannot be disabled",
    required: true,
    contributions: &[panel(
        "test-required.expandable",
        "Required",
        50,
        Zone::Right,
        "assistant",
        OWN_HEADER,
    )],
};

/// A core contribution, `shell.panel`, which owns a sidebar panel.
pub static TEST_CORE: &[SystemContribution] = &[panel(
    "shell.panel",
    "Core panel",
    30,
    Zone::Left,
    "navigation",
    TITLED,
)];

pub static PANEL_A: &SystemContribution = &TEST_PLUGIN.contributions[0];
pub static PANEL_B: &SystemContribution = &TEST_PLUGIN.contributions[1];
pub static EXPANDABLE: &SystemContribution = &TEST_PLUGIN.contributions[2];
pub static WIDGET: &SystemContribution = &TEST_PLUGIN.contributions[3];
pub static SIDEBAR_EXPANDABLE: &SystemContribution = &TEST_PLUGIN.contributions[4];
pub static REQUIRED_EXPANDABLE: &SystemContribution = &REQUIRED_PLUGIN.contributions[0];
pub static CORE_PANEL: &SystemContribution = &TEST_CORE[0];

/// Dependencies whose catalog lists the shell's own contributions and `plugins`, and nothing
/// else. Panics on an invalid combination, which a test must not compose.
pub fn deps_for(plugins: &[&'static SystemPlugin], cx: &mut App) -> Deps {
    deps_for_core(&[], plugins, cx)
}

/// Like [`deps_for`], with the core contributions of other owners.
pub fn deps_for_core(
    core: &[&'static [SystemContribution]],
    plugins: &[&'static SystemPlugin],
    cx: &mut App,
) -> Deps {
    let catalog = plugin::catalog(core, plugins).expect("the test plugins form a valid catalog");
    Deps::new(nocterm_ui::LayoutHandle::in_memory(cx)).with_catalog(catalog)
}

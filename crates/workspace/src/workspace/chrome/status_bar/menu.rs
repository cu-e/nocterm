//! The context menu of the status bar: hide the item under the pointer, open the layout editor
//! on it, show, hide or move the toolbars, and switch layouts.

use std::{cell::RefCell, rc::Rc};

use gpui_kit::{
    Context, SharedString, WeakEntity, Window,
    component::{
        Side,
        menu::{PopupMenu, PopupMenuItem},
    },
};
use nocterm_plugin_model::{ContributionId, Edge, PluginSet};
use nocterm_ui::{IconName, contribution_icon};
use nocterm_window_layout::EditOp;

use super::BarEntry;
use crate::{
    LayoutChoice, OpenLayoutSettings, SwitchLayout, Workspace,
    workspace::{chrome::ToolbarChoice, layout_edit},
};

/// The item the pointer was last pressed on, shared by the row that records it and the menu
/// that reads it.
#[derive(Clone, Default)]
pub(in crate::workspace) struct Pointed(Rc<RefCell<Option<ContributionId>>>);

impl Pointed {
    pub(in crate::workspace) fn set(&self, id: Option<ContributionId>) {
        *self.0.borrow_mut() = id;
    }

    fn get(&self) -> Option<ContributionId> {
        self.0.borrow().clone()
    }
}

/// An item of the bar the menu can hide.
struct Hideable {
    id: ContributionId,
    title: String,
    pinned: bool,
}

/// What the menu offers for the bar as drawn now.
pub(in crate::workspace) struct BarMenu {
    pointed: Pointed,
    items: Vec<Hideable>,
    layouts: Vec<LayoutChoice>,
    toolbars: Vec<ToolbarChoice>,
    workspace: WeakEntity<Workspace>,
}

impl BarMenu {
    pub(in crate::workspace) fn new<'a>(
        pointed: Pointed,
        entries: impl IntoIterator<Item = &'a BarEntry>,
        catalog: &PluginSet,
        layouts: Vec<LayoutChoice>,
        workspace: WeakEntity<Workspace>,
    ) -> Self {
        let items = entries
            .into_iter()
            .filter_map(BarEntry::id)
            .filter_map(|id| catalog.contribution(&id))
            .map(|contribution| Hideable {
                id: contribution.id().clone(),
                title: contribution.title().to_owned(),
                pinned: contribution.pinned(),
            })
            .collect();
        Self {
            pointed,
            items,
            layouts,
            toolbars: Vec::new(),
            workspace,
        }
    }

    /// The toolbars of the window, offered to show, hide and move.
    pub(in crate::workspace) fn with_toolbars(mut self, toolbars: Vec<ToolbarChoice>) -> Self {
        self.toolbars = toolbars;
        self
    }

    pub(in crate::workspace) fn build(
        &self,
        mut menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let target = self.pointed.get();
        let hideable = target
            .as_ref()
            .and_then(|id| self.items.iter().find(|item| item.id == *id))
            .filter(|item| !item.pinned);
        if let Some(item) = hideable {
            let (workspace, id) = (self.workspace.clone(), item.id.clone());
            menu = menu
                .item(PopupMenuItem::new(format!("Hide {}", item.title)).on_click(
                    move |_, window, cx| {
                        let id = id.clone();
                        let _ = workspace.update(cx, |workspace, cx| {
                            layout_edit::hide(workspace, id, window, cx)
                        });
                    },
                ))
                .separator();
        }
        let select = target.map(|id| SharedString::from(id.as_str().to_owned()));
        menu = menu.menu(
            "Customize Status Bar\u{2026}",
            Box::new(OpenLayoutSettings { select }),
        );
        if !self.toolbars.is_empty() {
            menu = self.toolbars_menu(menu, window, cx);
        }
        let layouts = self.layouts.clone();
        menu.submenu("Layouts", window, cx, move |mut submenu, _, _| {
            for choice in &layouts {
                let switch = SwitchLayout {
                    id: choice.id().as_str().to_owned().into(),
                };
                submenu = submenu.menu_with_check(
                    choice.name().to_owned(),
                    choice.is_selected(),
                    Box::new(switch),
                );
            }
            submenu
        })
    }

    /// "Toolbars": for each toolbar, under its own icon, where it runs, the edges it can run
    /// along and Hidden, each with its icon and the current one checked.
    fn toolbars_menu(
        &self,
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let toolbars = self.toolbars.clone();
        let workspace = self.workspace.clone();
        menu.submenu_with_icon(
            Some(IconName::PanelTop.into()),
            "Toolbars",
            window,
            cx,
            move |mut submenu, window, cx| {
                for toolbar in &toolbars {
                    let (toolbar, workspace) = (toolbar.clone(), workspace.clone());
                    let icon = contribution_icon(toolbar.icon).into();
                    submenu = submenu.submenu_with_icon(
                        Some(icon),
                        toolbar.title.clone(),
                        window,
                        cx,
                        move |edges, _, _| edges_menu(edges, &toolbar, &workspace),
                    );
                }
                submenu
            },
        )
    }
}

/// The edges a toolbar can run along and Hidden, under a line that says which tabs it runs
/// along.
fn edges_menu(
    mut edges: PopupMenu,
    toolbar: &ToolbarChoice,
    workspace: &WeakEntity<Workspace>,
) -> PopupMenu {
    edges = edges.check_side(Side::Right).label(if toolbar.own_tabs {
        "On its plugin's tabs"
    } else {
        "On every tab"
    });
    for edge in Edge::ALL {
        let op = EditOp::ShowToolbar {
            id: toolbar.id.clone(),
            edge,
        };
        edges = edges.item(edit_item(
            edge_label(edge),
            edge_icon(edge),
            toolbar.shown == Some(edge),
            op,
            workspace.clone(),
        ));
    }
    let hide = EditOp::RemoveToTray {
        id: toolbar.id.clone(),
    };
    edges.separator().item(edit_item(
        "Hidden",
        IconName::EyeOff,
        toolbar.shown.is_none(),
        hide,
        workspace.clone(),
    ))
}

/// A checked entry that applies `op` to the selected layout.
fn edit_item(
    label: &'static str,
    icon: IconName,
    checked: bool,
    op: EditOp,
    workspace: WeakEntity<Workspace>,
) -> PopupMenuItem {
    PopupMenuItem::new(label)
        .icon(icon)
        .checked(checked)
        .on_click(move |_, window, cx| {
            let op = op.clone();
            let _ = workspace.update(cx, |workspace, cx| {
                layout_edit::apply(workspace, op, window, cx)
            });
        })
}

fn edge_label(edge: Edge) -> &'static str {
    match edge {
        Edge::Top => "Top",
        Edge::Bottom => "Bottom",
        Edge::Left => "Left",
        Edge::Right => "Right",
    }
}

fn edge_icon(edge: Edge) -> IconName {
    match edge {
        Edge::Top => IconName::PanelTop,
        Edge::Bottom => IconName::PanelBottom,
        Edge::Left => IconName::PanelLeft,
        Edge::Right => IconName::PanelRight,
    }
}

//! The context menu of the status bar: hide the item under the pointer, open the layout editor
//! on it, and switch layouts.

use std::{cell::RefCell, rc::Rc};

use gpui_kit::{
    Context, SharedString, WeakEntity, Window,
    component::menu::{PopupMenu, PopupMenuItem},
};
use nocterm_plugin_model::{ContributionId, PluginCatalog};

use super::BarEntry;
use crate::{LayoutChoice, OpenLayoutSettings, SwitchLayout, Workspace, workspace::layout_edit};

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
    title: &'static str,
    pinned: bool,
}

/// What the menu offers for the bar as drawn now.
pub(in crate::workspace) struct BarMenu {
    pointed: Pointed,
    items: Vec<Hideable>,
    layouts: Vec<LayoutChoice>,
    workspace: WeakEntity<Workspace>,
}

impl BarMenu {
    pub(in crate::workspace) fn new<'a>(
        pointed: Pointed,
        entries: impl IntoIterator<Item = &'a BarEntry>,
        catalog: &PluginCatalog,
        layouts: Vec<LayoutChoice>,
        workspace: WeakEntity<Workspace>,
    ) -> Self {
        let items = entries
            .into_iter()
            .filter_map(BarEntry::id)
            .filter_map(|id| catalog.contribution(&id))
            .map(|contribution| Hideable {
                id: contribution.id.clone(),
                title: contribution.title,
                pinned: contribution.pinned,
            })
            .collect();
        Self {
            pointed,
            items,
            layouts,
            workspace,
        }
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
}

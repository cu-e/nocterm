//! Each window owns its closed snapshot and retains its original popup target.
use super::super::{ContextSelectionReceipt, MenuBuilder};
use gpui_kit::{
    App, Entity, EntityId, FocusHandle, OwnedMenu, Window, component::menu::AppMenuBar,
};

pub(super) struct WindowMenus {
    builder: Option<MenuBuilder>,
    bar: Option<Entity<AppMenuBar>>,
    focus: FocusHandle,
    target: Option<EntityId>,
    selection: Option<ContextSelectionReceipt>,
}
impl WindowMenus {
    pub(super) fn new(focus: FocusHandle) -> Self {
        Self {
            builder: None,
            bar: None,
            focus,
            target: None,
            selection: None,
        }
    }
    pub(super) fn install(&mut self, builder: MenuBuilder) {
        self.builder = Some(builder);
    }
    pub(super) fn builder(&self) -> Option<MenuBuilder> {
        self.builder.clone()
    }
    pub(super) fn focus(&self) -> &FocusHandle {
        &self.focus
    }
    pub(super) fn bar(&self) -> Option<Entity<AppMenuBar>> {
        self.bar.clone()
    }
    pub(super) fn is_open(&self, cx: &App) -> bool {
        self.bar.as_ref().is_some_and(|bar| bar.read(cx).is_open())
    }
    pub(super) fn stale_selection(
        &self,
        selection: ContextSelectionReceipt,
        window: &Window,
        cx: &App,
    ) -> bool {
        (self.is_open(cx) || self.focus.contains_focused(window, cx))
            && self.selection.is_some_and(|original| original != selection)
    }
    pub(super) fn command_target(
        &self,
        window: &Window,
        last: Option<EntityId>,
        cx: &App,
    ) -> Option<EntityId> {
        self.focus
            .contains_focused(window, cx)
            .then(|| self.target.or(last))
            .flatten()
    }
    pub(super) fn refresh(
        &mut self,
        menus: Vec<OwnedMenu>,
        target: Option<EntityId>,
        selection: ContextSelectionReceipt,
        cx: &mut App,
    ) {
        if self.is_open(cx) {
            return;
        }
        self.target = target;
        self.selection = Some(selection);
        let bar = self.bar.get_or_insert_with(|| AppMenuBar::new(cx));
        bar.update(cx, |bar, cx| bar.set_menus(menus, cx));
    }
    pub(super) fn clear_closed_target(&mut self, cx: &App) -> bool {
        if self.is_open(cx) {
            return false;
        }
        self.target = None;
        true
    }
}

//! Focus-sensitive command routing and window menu lifecycle.
use super::*;

impl Workspace {
    /// Focused visible user Item, then the last visible central context.
    /// An open menu retains its opening context while native popup focus moves.
    pub fn command_item(&self, window: &Window, cx: &App) -> Option<Rc<dyn ItemHandle>> {
        if let Some(open) = self.items.iter().find(|open| {
            open.dock_item.read(cx).contains_focus(window, cx)
                && self.item_visible(open.handle.item_id(), cx)
        }) {
            return Some(open.handle.clone());
        }
        if self.menu_focus.contains_focused(window, cx)
            && let Some(id) = self.menu_item.or(self.last_command_item)
        {
            return self
                .items
                .iter()
                .find(|open| open.handle.item_id() == id && self.item_visible(id, cx))
                .map(|open| open.handle.clone());
        }
        self.active_item
            .and_then(|ix| self.items.get(ix))
            .filter(|item| self.item_visible(item.handle.item_id(), cx))
            .map(|item| item.handle.clone())
    }

    pub fn item_command_enabled(&self, command: ItemCommand, window: &Window, cx: &App) -> bool {
        self.command_item(window, cx)
            .is_some_and(|item| item.command_enabled(command, cx))
    }

    pub fn execute_item_command(
        &mut self,
        command: ItemCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self.command_item(window, cx)
            && item.command_enabled(command, cx)
        {
            item.execute(command, window, cx);
        }
    }

    pub(super) fn dispatch_item_command(
        &mut self,
        command: ItemCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self.command_item(window, cx)
            && item.command_enabled(command, cx)
        {
            if matches!(
                command,
                ItemCommand::Find | ItemCommand::FindNextSelection | ItemCommand::SessionSettings
            ) {
                let workspace = cx.weak_entity();
                window.defer(cx, move |window, cx| {
                    let _ = workspace.update(cx, |this, cx| {
                        let still_open = this
                            .items
                            .iter()
                            .any(|open| open.handle.item_id() == item.item_id());
                        if still_open
                            && this.item_visible(item.item_id(), cx)
                            && item.command_enabled(command, cx)
                        {
                            item.execute(command, window, cx);
                        }
                    });
                });
            } else {
                item.execute(command, window, cx);
            }
        }
    }

    /// The displayed tab label, including its user-defined alias.
    pub fn command_title(&self, window: &Window, cx: &App) -> Option<SharedString> {
        let item = self.command_item(window, cx)?;
        let dock = self
            .items
            .iter()
            .find(|open| open.handle.item_id() == item.item_id())
            .map(|open| &open.dock_item);
        Some(
            dock.and_then(|dock| dock.read(cx).alias.clone())
                .unwrap_or_else(|| item.tab_title(cx)),
        )
    }

    pub fn sidebar_is_open(&self) -> bool {
        self.sidebar_open
    }
    /// Supplies window-specific menus to the toolkit's standard menu bar.
    pub fn set_menu_builder(
        &mut self,
        builder: impl Fn(&Workspace, &Window, &App) -> Vec<Menu> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu_builder = Some(Rc::new(builder));
        self.reload_menu_bar(window, cx);
    }

    fn reload_menu_bar(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(builder) = self.menu_builder.clone() else {
            return;
        };
        self.menu_item = self.command_item(window, cx).map(|item| item.item_id());
        let menus = builder(self, window, cx)
            .into_iter()
            .map(Menu::owned)
            .collect();
        if let Some(bar) = &self.app_menu_bar {
            bar.update(cx, |bar, cx| bar.set_menus(menus, cx));
        } else {
            // Construction reads GlobalState once; restore it immediately so
            // other windows and OS menus never inherit this snapshot.
            let previous = GlobalState::global(cx).app_menus().to_vec();
            GlobalState::global_mut(cx).set_app_menus(menus);
            self.app_menu_bar = Some(AppMenuBar::new(cx));
            GlobalState::global_mut(cx).set_app_menus(previous);
        }
    }

    pub(super) fn prepare_menu(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self
            .app_menu_bar
            .as_ref()
            .is_some_and(|bar| bar.read(cx).is_open())
        {
            self.menu_item = None;
            self.reload_menu_bar(window, cx);
        }
    }
}

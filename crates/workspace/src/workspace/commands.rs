//! Focus-sensitive command routing and window menu lifecycle.
use super::*;
use gpui_kit::{Action, InteractiveElement, component::input};

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

    /// Whether the command item answers `action` now. An Item registers a
    /// handler only while the command can run, so the last rendered frame is
    /// the one source of truth for menus, buttons and shortcuts.
    pub fn command_available(&self, action: &dyn Action, window: &Window, cx: &App) -> bool {
        // From the workspace root itself no forwarder reaches the item.
        !self.focus_handle.is_focused(window)
            && self
                .command_item(window, cx)
                .is_some_and(|item| window.is_action_available_in(action, &item.focus_handle(cx)))
    }

    /// Runs `action` on the command item after the current dispatch, so its
    /// handler can move focus freely and update the workspace.
    pub fn dispatch_command(
        &mut self,
        action: Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.command_item(window, cx) else {
            return;
        };
        let workspace = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let target = workspace.read_with(cx, |this, cx| {
                this.items
                    .iter()
                    .any(|open| open.handle.item_id() == item.item_id())
                    && this.item_visible(item.item_id(), cx)
            });
            if target.unwrap_or(false) {
                let focus = item.focus_handle(cx);
                if window.is_action_available_in(action.as_ref(), &focus) {
                    focus.dispatch_action(action.as_ref(), window, cx);
                }
            }
        });
    }

    /// Hands the commands an Item answers from the workspace's chrome to the
    /// command item. Never put this on an ancestor of the Items: a handler
    /// there would make every command look available.
    pub(super) fn forward_commands<E: InteractiveElement>(element: E, cx: &mut Context<Self>) -> E {
        fn forward<A: Action, E: InteractiveElement>(element: E, cx: &mut Context<Workspace>) -> E {
            element.on_action(cx.listener(|this, action: &A, window, cx| {
                this.dispatch_command(action.boxed_clone(), window, cx)
            }))
        }
        let element = forward::<crate::ClearSelection, _>(element, cx);
        let element = forward::<crate::Find, _>(element, cx);
        let element = forward::<crate::FindNext, _>(element, cx);
        let element = forward::<crate::FindPrevious, _>(element, cx);
        let element = forward::<crate::FindNextSelection, _>(element, cx);
        let element = forward::<crate::DisconnectSession, _>(element, cx);
        let element = forward::<crate::ReconnectSession, _>(element, cx);
        let element = forward::<crate::StartRecording, _>(element, cx);
        let element = forward::<crate::StopRecording, _>(element, cx);
        let element = forward::<crate::SessionSettings, _>(element, cx);
        let element = forward::<input::Copy, _>(element, cx);
        let element = forward::<input::Paste, _>(element, cx);
        forward::<input::SelectAll, _>(element, cx)
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

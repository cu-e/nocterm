//! Disposable configuration-search and usage-card presentation lifetimes.
use super::{AgentPanel, MenuKind};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, SharedString, Subscription, Window,
    component::{
        Sizable as _,
        input::{Input, InputEvent, InputState},
    },
};

pub(super) struct MenuView {
    menu: Option<MenuKind>,
    usage_focus: FocusHandle,
    search: Entity<InputState>,
    _bindings: Vec<Subscription>,
}
impl MenuView {
    pub(super) fn new(
        composer_focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search models…"));
        let usage_focus = cx.focus_handle();
        let bindings = vec![
            cx.subscribe(&search, |panel, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    panel.on_config_query_changed(cx);
                }
            }),
            cx.on_focus_out(&usage_focus, window, |panel, _, _, cx| {
                panel.on_usage_dismiss_requested(cx);
            }),
            cx.on_focus_out(&composer_focus, window, |panel, _, window, cx| {
                panel.on_usage_focus_changed(window, cx);
            }),
        ];
        Self {
            menu: None,
            usage_focus,
            search,
            _bindings: bindings,
        }
    }
    pub(super) fn current(&self) -> Option<&MenuKind> {
        self.menu.as_ref()
    }
    pub(super) fn is_open(&self, kind: &MenuKind) -> bool {
        self.menu.as_ref() == Some(kind)
    }
    pub(super) fn open(&mut self, kind: MenuKind) {
        self.menu = Some(kind);
    }
    pub(super) fn reset(&mut self) {
        self.menu = None;
    }
    pub(super) fn close_if_same(&mut self, kind: &MenuKind) -> bool {
        if !self.is_open(kind) {
            return false;
        }
        self.reset();
        true
    }
    pub(super) fn toggle(&mut self, kind: MenuKind) {
        if !self.close_if_same(&kind) {
            self.open(kind);
        }
    }
    pub(super) fn dismiss_usage(&mut self) -> bool {
        self.close_if_same(&MenuKind::Usage)
    }
    pub(super) fn usage_focus(&self) -> &FocusHandle {
        &self.usage_focus
    }
    pub(super) fn usage_contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.usage_focus.contains_focused(window, cx)
    }
    pub(super) fn query(&self, cx: &App) -> SharedString {
        self.search.read(cx).value()
    }
    pub(super) fn query_input(&self) -> Input {
        Input::new(&self.search).small()
    }
}

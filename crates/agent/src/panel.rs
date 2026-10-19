use crate::thread::AgentThread;
use gpui_kit::{
    AnyElement, App, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, TestSupportExt as _, Window,
    component::{ActiveTheme as _, v_flex},
    div,
    prelude::*,
};
use nocterm_ai::model::controls::ConfigCategory;
use nocterm_ui::IconName;
use nocterm_workspace::{Panel, RightPanel, RightPanelEvent, WorkspaceEvent};

mod approvals;
mod attach;
mod attachments;
mod bindings;
mod chat;
mod chat_list;
pub(crate) mod commands;
mod composer;
mod composer_renderer;
mod composer_view;
mod entries;
mod history;
mod history_column;
mod image_picker;
mod image_pipeline;
mod image_preparation;
mod images;
mod layout;
mod lifecycle;
mod menu;
mod menu_view;
mod message_actions;
mod message_list;
mod navigation;
mod notices;
mod preview_operation;
mod queue;
mod queue_image_preparation;
mod servers;
mod shell;
mod sign_in;
mod stream;
mod transcript;
mod usage;
mod widgets;

#[derive(Clone, PartialEq, Eq)]
enum MenuKind {
    Agents,
    Context,
    Config(String),
    Modes,
    /// The context usage card, over the chat rather than in a popover.
    Usage,
}

pub(crate) struct AgentPanel {
    focus: FocusHandle,
    host: crate::chat_host::ChatHost,
    chats: chat_list::ChatList,
    sign_ins: sign_in::SignInCards,
    notices: notices::PanelNotices,
    _bindings: bindings::PanelBindings,
    messages: message_list::MessageList,
    composer: composer_view::ComposerView,
    history: history_column::HistoryColumn,
    layout: layout::PanelLayout,
    menus: menu_view::MenuView,
    settings: nocterm_store::SettingsHandle,
}
impl EventEmitter<RightPanelEvent> for AgentPanel {}

#[cfg(test)]
mod tests;

impl AgentPanel {
    pub(crate) fn new(
        host: crate::chat_host::ChatHost,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let images = host.images().clone();
        let layout = host.layout().clone();
        let settings = host.settings().clone();
        let composer = composer_view::ComposerView::new(window, cx);
        let menus = menu_view::MenuView::new(composer.focus(cx), window, cx);
        let history = history_column::HistoryColumn::new(window, cx);
        let layout = layout::PanelLayout::new(layout, cx);
        let messages = message_list::MessageList::new(images, cx);
        let subscriptions = Self::bind_panel(&host, &settings, window, cx);
        let panel = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let _ = panel.update(cx, |panel, cx| panel.on_runtime_changed(window, cx));
        });
        Self {
            focus: cx.focus_handle(),
            host,
            chats: chat_list::ChatList::default(),
            notices: notices::PanelNotices::default(),
            _bindings: bindings::PanelBindings::new(subscriptions),
            sign_ins: sign_in::SignInCards::default(),
            messages,
            composer,
            history,
            layout,
            menus,
            settings,
        }
    }
    fn bind_panel(
        host: &crate::chat_host::ChatHost,
        settings: &nocterm_store::SettingsHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        let panel = cx.weak_entity();
        let originating_window = window.window_handle();
        let settings = settings.observe(cx, move |cx| {
            let _ = cx.update_window(originating_window, |_, window, cx| {
                let _ = panel.update(cx, |panel, cx| panel.on_panel_settings_changed(window, cx));
            });
        });
        let workspace = host.workspace().upgrade().map(|workspace| {
            cx.subscribe_in(&workspace, window, |panel, _, event, window, cx| {
                panel.on_workspace_event(*event, window, cx);
            })
        });
        let panel = cx.weak_entity();
        let runtime = host.runtime().observe(cx, move |cx| {
            let _ = cx.update_window(originating_window, |_, window, cx| {
                let _ = panel.update(cx, |panel, cx| panel.on_runtime_changed(window, cx));
            });
        });
        [Some(settings), workspace, Some(runtime)]
            .into_iter()
            .flatten()
            .collect()
    }
    fn on_runtime_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.adopt_saved_chats(window, cx);
        cx.notify();
    }
    fn on_workspace_event(
        &mut self,
        event: WorkspaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event == WorkspaceEvent::RightPanelVisibilityChanged {
            let visible = self.history_is_visible(cx);
            self.history.set_visible(visible, cx);
            if !self
                .host
                .workspace()
                .upgrade()
                .is_some_and(|workspace| workspace.read(cx).right_panel_is_open())
            {
                self.sync_composer_draft(cx);
                if let Some(thread) = self.current() {
                    thread.update(cx, |thread, cx| thread.flush_draft(cx));
                }
            }
            self.sync_approval_attention(window, cx);
        }
        cx.notify();
    }
    fn on_panel_settings_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let enabled = self.settings.setting::<nocterm_ai::AiSettings>(cx).enabled;
        if !enabled {
            self.clear_chats(window, cx);
        }
        let workspace = self.host.workspace().clone();
        window.defer(cx, move |window, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.set_right_panel_available(enabled, window, cx)
            });
        });
        cx.notify();
    }
    fn on_history_clock_tick(
        &mut self,
        ticket: &history_column::HistoryClockTicket,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.history.clock_current(ticket) {
            return false;
        }
        if !self.history_is_visible(cx) {
            self.history.stop_clock(ticket);
            return false;
        }
        cx.notify();
        true
    }
    fn history_is_visible(&self, cx: &App) -> bool {
        self.layout.history()
            && !self.chats.is_empty()
            && self
                .host
                .workspace()
                .upgrade()
                .is_some_and(|workspace| workspace.read(cx).right_panel_is_open())
    }
    fn on_history_split_settling(
        &mut self,
        slot: usize,
        ticket: &std::sync::Arc<()>,
        cx: &mut Context<Self>,
    ) {
        if self.history_is_visible(cx) && self.layout.settling_current(slot, ticket) {
            cx.notify();
        }
    }
    fn on_history_split_resized(
        &mut self,
        _index: usize,
        size: gpui_kit::Pixels,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.layout.on_split_resized(size, cx) {
            self.notices.set_error(Some(error.to_string()));
            cx.notify();
        }
    }
    fn on_shared_history_width_changed(
        &mut self,
        value: Option<gpui_kit::Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.layout.on_shared_width_changed(value, cx) {
            cx.notify();
        }
    }
    pub(crate) fn current(&self) -> Option<Entity<AgentThread>> {
        self.chats.current()
    }
    pub(crate) fn chat_documents(&self) -> impl ExactSizeIterator<Item = &Entity<AgentThread>> {
        self.chats.iter()
    }
    pub(crate) fn composer_text(&self, cx: &App) -> SharedString {
        self.composer.text(cx)
    }
    pub(crate) fn composer_focus(&self, cx: &App) -> FocusHandle {
        self.composer.focus(cx)
    }
    pub(crate) fn focus_composer(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.composer_focus(cx), cx);
    }
    /// The agent `NewThreadWithLastAgent` starts: the last one used, else
    /// the default, else the first enabled one.
    fn last_agent(&self, cx: &App) -> Option<String> {
        let registry =
            nocterm_ai::AgentRegistry::new(self.settings.setting::<nocterm_ai::AiSettings>(cx));
        let known = |id: &&String| registry.get(id).is_some();
        self.host
            .runtime()
            .favorites(cx)
            .last_agent
            .as_ref()
            .filter(known)
            .or(self
                .settings
                .setting::<nocterm_ai::AiSettings>(cx)
                .default_agent
                .as_ref()
                .filter(known))
            .cloned()
            .or_else(|| registry.iter().next().map(|launch| launch.id.clone()))
    }
    pub(super) fn set_history(&mut self, open: bool, cx: &mut Context<Self>) {
        if let Some(width) = self.layout.set_history(open, self.current().is_some(), cx) {
            cx.emit(RightPanelEvent::Widen(width));
        }
        cx.notify();
    }
    pub(super) fn render_split(
        &mut self,
        chat: AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let history = history::HistoryProjection::render(
            &self.chats,
            &self.history,
            self.host.timestamp(),
            cx,
        );
        self.layout.render_split(chat, history, window, cx)
    }
    fn toggle_history(&mut self, cx: &mut Context<Self>) {
        self.set_history(!self.layout.history(), cx);
    }
    fn new_thread_with_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.last_agent(cx) {
            self.new_thread(id, window, cx);
        }
    }
    fn stop_generation(&mut self, cx: &mut Context<Self>) {
        if let Some(thread) = self.current() {
            thread.update(cx, |thread, cx| thread.stop(cx));
        }
    }
    fn toggle_model_picker(&mut self, cx: &mut Context<Self>) {
        if let Some(thread) = self.current()
            && let Some(option) = thread
                .read(cx)
                .config_options()
                .iter()
                .find(|option| option.category == Some(ConfigCategory::Model))
        {
            self.toggle_menu(MenuKind::Config(option.id.to_string()), cx);
        }
    }
}

impl AgentPanel {
    pub(super) fn render_menu(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let kind = self.menus.current().cloned();
        let attachment = matches!(kind, Some(MenuKind::Context)).then(|| {
            attach::AttachmentMenuRenderer::render_attach_menu(
                self.current(),
                self.host.workspace().clone(),
                cx,
            )
        });
        let frame = menu::MenuFrame::new(
            kind,
            self.current(),
            self.menus.query_input(),
            self.menus.query(cx).to_lowercase(),
            self.settings.setting::<nocterm_ai::AiSettings>(cx).clone(),
            self.last_agent(cx),
            self.focus.clone(),
            self.host.runtime().favorites(cx).clone(),
            attachment,
        );
        menu::ConfigMenuRenderer::render_menu(frame, window, cx)
    }
}

impl AgentPanel {
    pub(super) fn on_config_query_changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }
}

impl Focusable for AgentPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Panel for AgentPanel {
    fn title(&self, cx: &App) -> SharedString {
        self.current()
            .map(|thread| thread.read(cx).title())
            .unwrap_or_else(|| "New Agent".into())
            .into()
    }
    fn icon(&self, _: &App) -> IconName {
        IconName::Bot
    }
}

impl RightPanel for AgentPanel {
    fn set_maximized(&mut self, maximized: bool, cx: &mut Context<Self>) {
        self.layout.set_maximized(maximized);
        cx.notify();
    }
    fn set_docked_left(&mut self, left: bool, cx: &mut Context<Self>) {
        self.layout.set_docked_left(left);
        cx.notify();
    }
}

impl Render for AgentPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current();
        self.history.set_visible(self.history_is_visible(cx), cx);
        let agent = current
            .as_ref()
            .map(|thread| thread.read(cx).document().agent_id().to_owned());
        let header = shell::ShellRenderer::header(
            agent,
            self.title(cx),
            self.layout.history(),
            self.layout.maximized(),
            &self.menus,
            cx,
        );
        let mut body = shell::ShellRenderer::body(&self.focus, cx).child(header);
        body = match current {
            None => body.child(history::HistoryProjection::render(
                &self.chats,
                &self.history,
                self.host.timestamp(),
                cx,
            )),
            Some(thread) if self.layout.history() => {
                let chat = self.render_chat(thread, window, cx);
                body.child(self.render_split(chat, window, cx))
            }
            Some(thread) => body.child(self.render_chat(thread, window, cx)),
        };
        if let Some(error) = self.notices.error() {
            body = body.child(
                shell::ShellRenderer::error(error.to_owned(), cx)
                    .id("agent-error")
                    .test_support(),
            );
        }
        if let Some(error) = self.host.runtime().favorites_error(cx) {
            body = body.child(shell::ShellRenderer::error(
                format!("Favorites were not saved: {error}"),
                cx,
            ));
        }
        body
    }
}

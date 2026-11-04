use crate::chat_host::ChatHostError;
use crate::thread::AgentThread;
use account_notice::AccountNotice;
use gpui_kit::{
    AnyElement, App, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, TestSupportExt as _, Window, component::v_flex, div, prelude::*,
};
use nocterm_ai::model::controls::ConfigCategory;
use nocterm_cloud_agents::StartId;
use nocterm_cloud_agents_model::{CloudChatError, OpenedChat};
use nocterm_plugin_model::Side;
use nocterm_ui::IconName;
use nocterm_workspace::{ExpandablePanel, Panel, PanelRequest, WorkspaceEvent};

mod account_notice;
mod approvals;
mod attach;
mod attachments;
mod bindings;
mod chat;
mod cloud_history;
mod cloud_menu;
pub(crate) mod commands;
mod composer;
mod composer_renderer;
mod composer_view;
mod conversation_surface;
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
mod message_markdown;
mod navigation;
mod notices;
mod parts;
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
    chats: crate::roster::ChatRoster,
    sign_ins: sign_in::SignInCards,
    notices: notices::PanelNotices,
    _bindings: bindings::PanelBindings,
    messages: message_list::MessageList,
    composer: composer_view::ComposerView,
    history: history_column::HistoryColumn,
    layout: layout::PanelLayout,
    menus: menu_view::MenuView,
    settings: nocterm_store::SettingsHandle,
    source: nocterm_workspace::ProjectSurfaceReceipt,
    conversation: Option<conversation_surface::ConversationSurface>,
    cloud: cloud_menu::CloudMenu,
}
impl EventEmitter<PanelRequest> for AgentPanel {}

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
        let cloud = cloud_menu::CloudMenu::new(host.cloud_agents().cloned(), window, cx);
        let history = history_column::HistoryColumn::new(host.cloud_agents().cloned(), window, cx);
        let layout = layout::PanelLayout::new(layout, cx);
        let messages = message_list::MessageList::new(images, cx);
        let subscriptions = Self::bind_panel(&host, &settings, window, cx);
        // Whatever the panel redraws for, its cached regions redraw too; thread changes
        // and typing redraw only their regions instead of notifying the panel.
        cx.observe_self(|panel, cx| panel.layout.parts().invalidate(cx))
            .detach();
        let panel = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let _ = panel.update(cx, |panel, cx| panel.on_runtime_changed(window, cx));
        });
        Self {
            focus: cx.focus_handle(),
            chats: crate::roster::ChatRoster::new(host.deps().clone()),
            source: host.source(),
            host,
            notices: notices::PanelNotices::default(),
            _bindings: bindings::PanelBindings::new(subscriptions),
            sign_ins: sign_in::SignInCards::default(),
            messages,
            composer,
            history,
            layout,
            menus,
            settings,
            conversation: None,
            cloud,
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
        let retired = self.chats.retire_stale_sources(cx);
        let removed = !retired.removed().is_empty();
        if let Some(error) = retired.into_error() {
            self.notices.set_error(Some(error.to_string()));
        }
        let deps = self.host.deps_for_source(self.source);
        let account_notice = match &deps {
            Ok(_) => None,
            Err(ChatHostError::Account(error)) => AccountNotice::from_capture(error),
            Err(ChatHostError::WindowMismatch) => None,
        };
        self.notices.set_account(account_notice);
        let switched = deps
            .ok()
            .is_some_and(|deps| self.chats.switch_context(deps, cx));
        if switched || removed {
            if switched && let Some(previous) = self.conversation.take() {
                previous.detach(cx);
            }
            self.composer.clear();
            self.history.switch_context(window, cx);
            self.sign_ins.clear();
            self.reset_chat_view(cx);
        }
        self.chats.release_quiet_hidden(cx);
        if !self.settings.setting::<nocterm_ai::AiSettings>(cx).enabled {
            return;
        }
        if account_notice.is_some() {
            cx.notify();
            return;
        }
        self.load_history(cx);
        self.history.cloud_mut().refresh(false, cx);
        self.adopt_saved_chats(window, cx);
        cx.notify();
    }
    /// Asks for the history of the captured context; a failure of the account becomes the
    /// account notice, any other the error line.
    fn load_history(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.chats.host().runtime().ensure_history(cx) {
            match AccountNotice::from_catalog(&error) {
                Some(notice) => self.notices.set_account(Some(notice)),
                None => self.notices.set_error(Some(error.to_string())),
            }
        }
    }
    fn select_context(
        &mut self,
        source: nocterm_workspace::ProjectSurfaceReceipt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if source == self.source {
            return;
        }
        let deps = match self.host.deps_for_source(source) {
            Ok(deps) => deps,
            Err(error) => {
                match &error {
                    ChatHostError::Account(cause) => match AccountNotice::from_capture(cause) {
                        Some(notice) => self.notices.set_account(Some(notice)),
                        None => self.notices.set_error(Some(error.to_string())),
                    },
                    ChatHostError::WindowMismatch => {
                        self.notices.set_error(Some(error.to_string()));
                    }
                }
                cx.notify();
                return;
            }
        };
        self.notices.set_account(None);
        self.leave_composer(cx);
        if let Some(previous) = self.conversation.take() {
            previous.detach(cx);
        }
        self.chats.switch_context(deps, cx);
        self.source = source;
        self.composer.clear();
        self.history.switch_context(window, cx);
        self.sign_ins.clear();
        self.notices.set_error(None);
        self.reset_chat_view(cx);
        if let Some(thread) = self.current() {
            let binding = self.composer.bind_loading(thread.entity_id(), cx);
            let archived = thread.read(cx).is_archived();
            let draft = thread.read(cx).draft_text().unwrap_or_default().to_owned();
            self.composer
                .finish_loading(&binding, draft, archived, window, cx);
        } else {
            self.composer.set_text("", window, cx);
        }
        self.load_history(cx);
        self.adopt_saved_chats(window, cx);
        self.sync_approval_attention(window, cx);
        cx.notify();
    }
    fn on_workspace_event(
        &mut self,
        event: WorkspaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event == WorkspaceEvent::ContextChanged {
            // This synchronous committed event captures the original source once.
            // Async history/runtime callbacks never consult current UI selection.
            if let Some(workspace) = self.host.workspace().upgrade() {
                let source = workspace.read(cx).context_source(cx);
                self.select_context(source, window, cx);
            }
        }
        if event == WorkspaceEvent::PanelVisibilityChanged {
            let visible = self.history_is_visible(cx);
            self.history.set_visible(visible, cx);
            if !self
                .host
                .workspace()
                .upgrade()
                .is_some_and(|workspace| workspace.read(cx).panel_is_open(&crate::plugin::CHATS.id))
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
                workspace.set_panel_available(&crate::plugin::CHATS.id, enabled, window, cx)
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
            && (!self.chats.is_empty() || !self.history.cloud().is_empty(cx))
            && self
                .host
                .workspace()
                .upgrade()
                .is_some_and(|workspace| workspace.read(cx).panel_is_open(&crate::plugin::CHATS.id))
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
        self.conversation
            .is_none()
            .then(|| self.chats.current())
            .flatten()
    }
    pub(super) fn conversation_identity(&self) -> Option<EntityId> {
        self.conversation
            .as_ref()
            .map(|view| view.identity())
            .or_else(|| self.current().map(|thread| thread.entity_id()))
    }
    #[cfg(test)]
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
        let agents = self.host.agents().available(cx);
        let known = |id: &&String| agents.get(id).is_some();
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
            .or_else(|| agents.iter().next().map(|agent| agent.id().to_owned()))
    }
    pub(super) fn set_history(&mut self, open: bool, cx: &mut Context<Self>) {
        if open {
            self.history.cloud_mut().refresh(true, cx);
        }
        if let Some(width) = self.layout.set_history(open, self.current().is_some(), cx) {
            cx.emit(PanelRequest::Widen(width));
        }
        cx.notify();
    }
    pub(super) fn render_split(
        &mut self,
        chat: AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let history = self.layout.parts().history().into_any_element();
        self.layout.render_split(chat, history, window, cx)
    }
    /// Draws one region of the panel for its view in [`parts`].
    fn render_part(
        &mut self,
        kind: parts::PartKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match kind {
            parts::PartKind::Transcript => self.render_transcript_part(cx),
            parts::PartKind::History => {
                let open = self.conversation.as_ref().and_then(|view| view.origin());
                let stamp = history::HistoryProjection::stamp(&self.chats, &self.history, open, cx);
                self.layout.parts_mut().drew_history(stamp);
                let history = history::HistoryProjection::render(
                    history::HistorySources::new(&self.chats, &self.history, open),
                    self.host.agents(),
                    self.host.timestamp(),
                    cx,
                );
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(history)
                    .into_any_element()
            }
            parts::PartKind::Footer => match (self.current(), self.conversation.is_some()) {
                (_, true) => self.render_conversation(window, cx),
                (Some(thread), false) => self.render_chat(thread, window, cx),
                (None, false) => div().into_any_element(),
            },
        }
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
        if self.conversation.is_some() {
            self.perform_conversation_action(
                nocterm_ai::conversation::ConversationAction::Stop,
                None,
                cx,
            );
            return;
        }
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
            self.host.agents().available(cx),
            self.last_agent(cx),
            self.focus.clone(),
            self.host.runtime().favorites(cx).clone(),
            attachment,
        );
        let (tabs, rows) = if matches!(frame.kind(), Some(MenuKind::Agents)) {
            let tabs = self.cloud.render_menu_tabs(cx);
            let rows = (self.cloud.tab(cx) == cloud_menu::MenuTab::Cloud)
                .then(|| self.cloud.render_tab(cx));
            (tabs, rows)
        } else {
            (None, None)
        };
        menu::ConfigMenuRenderer::render_menu(frame.with_cloud(tabs, rows), window, cx)
    }
}

impl AgentPanel {
    pub(super) fn on_config_query_changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    /// Opens the Servers page of the settings, where personal agents are created and edited.
    fn new_personal_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menus.reset();
        if let Ok(action) = cx.build_action("hubs::ManageServers", None) {
            window.dispatch_action(action, cx);
        }
        cx.notify();
    }

    /// Shows the chat a Cloud request opened, or tells the Cloud tab why it could not.
    fn finish_cloud_chat(
        &mut self,
        result: Result<OpenedChat, CloudChatError>,
        start: Option<StartId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.present_cloud_chat(result, window, cx) {
            None => {
                self.cloud.chat_opened(start, cx);
                self.menus.reset();
            }
            Some(notice) => self.cloud.chat_failed(notice, start),
        }
        cx.notify();
    }
    /// Shows the chat a history row opened, or says on the error line why it could not.
    fn finish_cloud_history_chat(
        &mut self,
        result: Result<OpenedChat, CloudChatError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(notice) = self.present_cloud_chat(result, window, cx) {
            self.notices.set_error(Some(notice));
        }
        cx.notify();
    }
    /// Presents an opened Cloud chat in this panel; its history row is the open one, and the
    /// history lists it at once if it is new. Returns what to tell the person when it could not.
    fn present_cloud_chat(
        &mut self,
        result: Result<OpenedChat, CloudChatError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let opened = match result {
            Ok(opened) => opened,
            Err(error) => return Some(cloud_menu::notice_text(&error)),
        };
        let source = match self.host.cloud_chats().open(opened.source, cx) {
            Ok(source) => source,
            Err(error) => return Some(error.to_string()),
        };
        if let Err(error) =
            self.open_conversation(source, SharedString::from(opened.title), window, cx)
        {
            return Some(error.to_string());
        }
        if let Some(open) = &mut self.conversation {
            open.set_origin(Some(opened.key));
        }
        self.history.cloud_mut().refresh(true, cx);
        None
    }
}

impl Focusable for AgentPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Panel for AgentPanel {
    fn title(&self, cx: &App) -> SharedString {
        if let Some(conversation) = &self.conversation {
            return conversation.title();
        }
        self.current()
            .map(|thread| thread.read(cx).title())
            .unwrap_or_else(|| "New Agent".into())
            .into()
    }
    fn icon(&self, _: &App) -> IconName {
        IconName::Bot
    }
    fn attention(&self, cx: &App) -> Option<SharedString> {
        let mut captured = false;
        if let Some(selected) = &self.conversation {
            let _ = selected.source().with_view(cx, &mut |view| {
                captured =
                    view.phase == nocterm_ai::conversation::ConversationPhase::WaitingApproval;
            });
        }
        (captured
            || self
                .chats
                .iter()
                .any(|thread| thread.read(cx).has_pending_approvals()))
        .then(|| "An agent needs permission to continue.".into())
    }
}

impl ExpandablePanel for AgentPanel {
    fn set_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        self.layout.set_maximized(expanded);
        cx.notify();
    }
    fn set_side(&mut self, side: Side, cx: &mut Context<Self>) {
        self.layout.set_docked_left(side == Side::Left);
        cx.notify();
    }
}

impl Render for AgentPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current();
        self.history.set_visible(self.history_is_visible(cx), cx);
        let agent = current.as_ref().map(|thread| {
            let id = thread.read(cx).document().agent_id();
            crate::agent_icon::agent_icon(self.host.agents().profile(id, cx))
        });
        let header = shell::ShellRenderer::header(
            agent,
            self.title(cx),
            self.layout.history(),
            self.layout.maximized(),
            &self.menus,
            cx,
        );
        let mut body = shell::ShellRenderer::body(&self.focus, cx).child(header);
        body = if self.conversation.is_none() && current.is_none() {
            body.child(self.layout.parts().history())
        } else {
            let chat = v_flex()
                .size_full()
                .min_w_0()
                .min_h_0()
                .child(self.layout.parts().transcript())
                .child(self.layout.parts().footer())
                .into_any_element();
            if self.layout.history() {
                body.child(self.render_split(chat, window, cx))
            } else {
                body.child(chat)
            }
        };
        if let Some(notice) = self.notices.account() {
            body = body.child(notice.strip(cx));
        }
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

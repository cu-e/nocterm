use crate::{runtime::Runtime, thread::AgentThread};
use gpui_kit::{
    Anchor, App, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Subscription,
    TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        floating::FloatingCards,
        h_flex, v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_ai::acp;
use nocterm_ui::IconName;
use nocterm_workspace::{Panel, RightPanel, RightPanelEvent, WorkspaceEvent};
use std::collections::HashMap;
use widgets::HEADER_HEIGHT;

mod approvals;
mod archive_search;
mod attach;
mod attachments;
mod chat;
pub(crate) mod commands;
mod composer;
mod composer_view;
mod entries;
mod history;
mod history_column;
mod image_pipeline;
mod images;
mod layout;
mod lifecycle;
mod menu;
mod menu_view;
mod message_actions;
mod message_list;
mod navigation;
mod queue;
mod servers;
mod split;
mod stream;
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
    threads: Vec<Entity<AgentThread>>,
    active: Option<usize>,
    error: Option<String>,
    subscriptions: Vec<Subscription>,
    notify_queued: bool,
    approval_notice: Option<Vec<(gpui_kit::EntityId, u64)>>,
    sign_in_inputs: HashMap<gpui_kit::EntityId, approvals::SignInInput>,
    messages: message_list::MessageList,
    composer: composer_view::ComposerView,
    history: history_column::HistoryColumn,
    layout: layout::PanelLayout,
    menus: menu_view::MenuView,
    settings: nocterm_store::SettingsHandle,
}
impl EventEmitter<RightPanelEvent> for AgentPanel {}
impl AgentPanel {
    pub(crate) fn new(
        host: crate::chat_host::ChatHost,
        deps: crate::PanelDeps,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (images, layout, settings) = deps.into_parts();
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
            threads: Vec::new(),
            active: None,
            error: None,
            subscriptions,
            notify_queued: false,
            approval_notice: None,
            sign_in_inputs: HashMap::new(),
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
        let runtime = cx.observe_in(&Runtime::global(cx), window, |panel, _, window, cx| {
            panel.on_runtime_changed(window, cx);
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
    fn on_composer_input_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.composer.observe_input(cx) {
            return;
        }
        if self.menus.is_open(&MenuKind::Usage) {
            self.close_usage(window, cx);
        }
        self.sync_composer_draft(cx);
        self.sync_command_token(window, cx);
        cx.notify();
    }
    fn submit_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.send(window, cx);
    }
    fn on_config_query_changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }
    fn on_usage_dismiss_requested(&mut self, cx: &mut Context<Self>) {
        self.dismiss_usage(cx);
    }
    fn on_usage_focus_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.menus.usage_contains_focus(window, cx) {
            self.dismiss_usage(cx);
        }
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
            && !self.threads.is_empty()
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
            self.error = Some(error.to_string());
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
        self.active
            .and_then(|index| self.threads.get(index))
            .cloned()
    }
    pub(crate) fn chat_documents(&self) -> impl ExactSizeIterator<Item = &Entity<AgentThread>> {
        self.threads.iter()
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
        Runtime::global(cx)
            .read(cx)
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
    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preparing_images() {
            return;
        }
        let Some(thread) = self.current() else {
            return;
        };
        let text = if self.composer.editing() {
            self.composer.text(cx).to_string()
        } else {
            self.original_composer_text(cx)
        };
        let replace = self.composer.replacement_for(thread.entity_id());
        let result = thread.update(cx, |thread, cx| thread.submit(text, replace, cx));
        match result {
            Ok(true) => {
                self.composer.set_text("", window, cx);
                self.finish_queue_edit(window, cx);
            }
            Ok(false) => return,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        }
        self.menus.reset();
        cx.notify();
    }
    fn toggle_menu(&mut self, menu: MenuKind, cx: &mut Context<Self>) {
        self.menus.toggle(menu);
        cx.notify();
    }
    fn sync_approval_attention(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let attention = self
            .threads
            .iter()
            .any(|thread| thread.read(cx).has_pending_approvals());
        let needs_notice = attention
            && (self
                .current()
                .is_none_or(|thread| !thread.read(cx).has_pending_approvals())
                || self
                    .host
                    .workspace()
                    .upgrade()
                    .is_none_or(|workspace| !workspace.read(cx).right_panel_is_open()));
        let _ = self.host.workspace().update(cx, |workspace, cx| {
            workspace.set_right_panel_attention(attention, cx)
        });
        let requests = needs_notice.then(|| {
            self.threads
                .iter()
                .filter_map(|thread| {
                    let state = thread.read(cx);
                    state
                        .has_pending_approvals()
                        .then_some((thread.entity_id(), state.approval_notice_generation()))
                })
                .collect()
        });
        if self.approval_notice == requests {
            return;
        }
        self.approval_notice = requests;
        if !needs_notice {
            nocterm_ui::notice::remove(window, cx, "agent-approval");
            return;
        }
        let panel = cx.weak_entity();
        let workspace = self.host.workspace().clone();
        nocterm_ui::notice::warning_action(
            window,
            cx,
            "agent-approval",
            "Agent request",
            "An agent needs permission to continue.",
            "Review",
            move |window, cx| {
                let _ = panel.update(cx, |panel, cx| panel.review_pending(cx));
                let _ = workspace.update(cx, |workspace, cx| {
                    if !workspace.right_panel_is_open() {
                        workspace.toggle_right_panel(window, cx);
                    }
                });
            },
        );
    }
    fn review_pending(&mut self, cx: &mut Context<Self>) {
        let pending = |thread: &Entity<AgentThread>| thread.read(cx).has_pending_approvals();
        let owner = self
            .current()
            .filter(pending)
            .or_else(|| self.threads.iter().find(|thread| pending(thread)).cloned());
        if let Some(owner) = owner {
            if self
                .current()
                .is_none_or(|thread| thread.entity_id() != owner.entity_id())
            {
                self.open_thread(owner.entity_id(), cx);
            }
            cx.notify();
        }
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
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current();
        self.history.set_visible(self.history_is_visible(cx), cx);
        let mut body = v_flex()
            .id("agent-panel")
            .test_support()
            .key_context("AgentPanel")
            .track_focus(&self.focus)
            .size_full()
            .min_w_0()
            // A floating card brings the panel's fill.
            .when(FloatingCards::get(cx).is_none(), |body| {
                body.bg(cx.theme().background)
            })
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::MoveUp, window, cx| {
                    this.command_action("up", window, cx);
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::MoveDown, window, cx| {
                    this.command_action("down", window, cx);
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::IndentInline, window, cx| {
                    this.command_action("tab", window, cx);
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::OutdentInline, window, cx| {
                    this.command_action("shift-tab", window, cx);
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::Escape, window, cx| {
                    if this.dismiss_usage(cx) {
                        cx.stop_propagation();
                    } else {
                        this.command_action("escape", window, cx);
                    }
                },
            ))
            .capture_action(cx.listener(
                |this, action: &gpui_kit::component::input::Enter, window, cx| {
                    if !action.shift && !action.secondary {
                        this.command_action("enter", window, cx);
                    }
                },
            ))
            .on_action(cx.listener(|this, _: &crate::NewThread, _, cx| {
                this.toggle_menu(MenuKind::Agents, cx)
            }))
            .on_action(
                cx.listener(|this, _: &crate::NewThreadWithLastAgent, window, cx| {
                    if let Some(id) = this.last_agent(cx) {
                        this.new_thread(id, window, cx);
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &crate::ShowHistory, _, cx| {
                this.set_history(!this.layout.history(), cx)
            }))
            .on_action(cx.listener(|this, _: &crate::StopGeneration, _, cx| {
                if let Some(thread) = this.current() {
                    thread.update(cx, |thread, cx| thread.stop(cx));
                }
            }))
            .on_action(cx.listener(|this, _: &crate::ToggleModelPicker, _, cx| {
                if let Some(thread) = this.current()
                    && let Some(option) = thread.read(cx).config_options().iter().find(|option| {
                        option.category == Some(acp::SessionConfigOptionCategory::Model)
                    })
                {
                    this.toggle_menu(MenuKind::Config(option.id.to_string()), cx);
                }
            }))
            .on_action(cx.listener(|this, _: &crate::AttachImage, _, cx| this.pick_images(cx)))
            .child(
                h_flex()
                    .id("agent-header")
                    .test_support()
                    .flex_shrink_0()
                    // As tall as the workspace's tab strip, so the two line up.
                    .h(px(HEADER_HEIGHT))
                    .gap_1()
                    .pl_3()
                    .pr_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .text_sm()
                    .child({
                        let agent = self
                            .current()
                            .map(|thread| thread.read(cx).document().agent_id().to_owned());
                        match agent {
                            Some(agent) => nocterm_ui::agent_icon(&agent).small(),
                            None => gpui_kit::component::Icon::new(IconName::Bot)
                                .small()
                                .text_color(cx.theme().muted_foreground),
                        }
                    })
                    .child(
                        div()
                            .id("agent-chat-title")
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .tooltip({
                                let title = self.title(cx);
                                move |_, cx| {
                                    cx.new(|_| {
                                        gpui_kit::component::tooltip::Tooltip::new(title.clone())
                                    })
                                    .into()
                                }
                            })
                            .child(widgets::single_line_label(&self.title(cx))),
                    )
                    .child(
                        Button::new("agent-history")
                            .ghost()
                            .small()
                            .icon(IconName::Clock)
                            .tooltip("Chat history")
                            .selected(self.layout.history())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_history(!this.layout.history(), cx)
                            })),
                    )
                    .child(
                        self.menu_popover(
                            "agent-new-popover",
                            MenuKind::Agents,
                            Button::new("agent-new-thread")
                                .ghost()
                                .small()
                                .icon(IconName::Plus)
                                .tooltip("New chat"),
                            Anchor::TopRight,
                            cx,
                        ),
                    )
                    .child(
                        Button::new("agent-maximize")
                            .ghost()
                            .small()
                            .icon(if self.layout.maximized() {
                                IconName::Minimize2
                            } else {
                                IconName::Maximize2
                            })
                            .tooltip(if self.layout.maximized() {
                                "Restore panel"
                            } else {
                                "Expand chat"
                            })
                            .on_click(
                                cx.listener(|_, _, _, cx| {
                                    cx.emit(RightPanelEvent::ToggleMaximized)
                                }),
                            ),
                    ),
            );
        body = match current {
            None => body.child(self.render_history(cx)),
            Some(thread) if self.layout.history() => {
                let chat = self.render_chat(thread, window, cx);
                body.child(self.render_split(chat, window, cx))
            }
            Some(thread) => body.child(self.render_chat(thread, window, cx)),
        };
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .p_2()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        if let Some(error) = &Runtime::global(cx).read(cx).favorites_error(cx) {
            body = body.child(
                div()
                    .p_2()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(format!("Favorites were not saved: {error}")),
            );
        }
        body
    }
}

#[cfg(test)]
mod tests;

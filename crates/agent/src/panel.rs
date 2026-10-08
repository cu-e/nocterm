use crate::{runtime::Runtime, thread::AgentThread};
use gpui_kit::{
    Anchor, App, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Subscription,
    Task, TestSupportExt as _, WeakEntity, Window,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        floating::FloatingCards,
        h_flex,
        input::{InputEvent, InputState, TextareaState},
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_ai::acp;
use nocterm_ui::{ActiveAi as _, ActiveSettings as _, IconName, SettingsStore};
use nocterm_workspace::{Panel, RightPanel, RightPanelEvent, Workspace, WorkspaceEvent};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use widgets::HEADER_HEIGHT;

mod approvals;
mod attach;
mod attachments;
mod chat;
pub(crate) mod commands;
mod composer;
mod entries;
mod history;
mod images;
mod lifecycle;
mod menu;
mod message_actions;
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
enum CachedImage {
    Loading(Option<Task<()>>),
    Ready(Arc<gpui_kit::Image>),
    Failed,
}
pub(crate) struct AgentPanel {
    image_cache: HashMap<(usize, usize, bool), CachedImage>,
    focus: FocusHandle,
    workspace: WeakEntity<Workspace>,
    threads: Vec<Entity<AgentThread>>,
    active: Option<usize>,
    history: bool,
    /// How much opening the history column widened the panel.
    widened: Option<gpui_kit::Pixels>,
    /// The chat and history split, with the history on the right and on the left.
    splits: [Entity<gpui_kit::component::ResizableState>; 2],
    /// Whether the panel is on the left of the tabs.
    docked_left: bool,
    maximized: bool,
    input: Entity<TextareaState>,
    search: Entity<InputState>,
    history_search: Entity<InputState>,
    renaming: Option<history::Renaming>,
    menu: Option<MenuKind>,
    usage_focus: FocusHandle,
    list: Entity<gpui_kit::component::message_scroller::MessageScrollerState>,
    stream: stream::StreamReveal,
    stream_tick: Option<Task<()>>,
    queue_open: bool,
    queue_heights: HashMap<gpui_kit::EntityId, gpui_kit::Pixels>,
    composer: queue::ComposerState,
    commands: commands::CommandState,
    list_count: usize,
    expanded: HashSet<usize>,
    error: Option<String>,
    subscriptions: Vec<Subscription>,
    notify_queued: bool,
    approval_notice: Option<Vec<(gpui_kit::EntityId, u64)>>,
    history_tick: Option<Task<()>>,
    /// Password fields of the sign-in cards, by background session.
    sign_in_inputs: HashMap<gpui_kit::EntityId, approvals::SignInInput>,
}
impl EventEmitter<RightPanelEvent> for AgentPanel {}
impl AgentPanel {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 10)
                .submit_on_enter(true)
                .placeholder("Ask about your terminals…")
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search models…"));
        let submit = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                this.send(window, cx);
            }
        });
        let search_observer = cx.subscribe(&search, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let input_observer = cx.observe_in(&input, window, |this, _, window, cx| {
            let value = this.input.read(cx).value().to_string();
            if value == this.commands.input_value {
                return;
            }
            if this.menu == Some(MenuKind::Usage) {
                this.close_usage(window, cx);
            }
            this.commands.input_value = value;
            if !this
                .commands
                .cycle
                .as_ref()
                .is_some_and(|cycle| cycle.inserted == this.input.read(cx).value().as_ref())
            {
                this.commands.cycle = None;
                this.commands.selected = 0;
                this.commands.reveal(0);
                this.commands.dismissed = None;
            }
            this.sync_command_token(window, cx);
            cx.notify();
        });
        let history_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search chats…"));
        let history_search_observer = cx.subscribe(&history_search, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let settings = cx.observe_global_in::<SettingsStore>(window, |this, window, cx| {
            let enabled = cx.ai_enabled();
            if !enabled {
                this.clear_chats(window, cx);
            }
            let workspace = this.workspace.clone();
            window.defer(cx, move |window, cx| {
                let _ = workspace.update(cx, |workspace, cx| {
                    workspace.set_right_panel_available(enabled, window, cx)
                });
            });
            cx.notify();
        });
        let workspace_observer = workspace.upgrade().map(|workspace| {
            cx.subscribe_in(
                &workspace,
                window,
                |this, _, event, window, cx| match event {
                    WorkspaceEvent::ItemsChanged
                    | WorkspaceEvent::ActiveItemChanged
                    | WorkspaceEvent::ActiveSessionChanged
                    | WorkspaceEvent::LocalDirectoryChanged
                    | WorkspaceEvent::ConnectionsChanged => cx.notify(),
                    WorkspaceEvent::RightPanelVisibilityChanged => {
                        this.sync_approval_attention(window, cx);
                        cx.notify();
                    }
                },
            )
        });
        let runtime_observer =
            cx.observe_in(&Runtime::global(cx), window, |this, _, window, cx| {
                this.adopt_saved_chats(window, cx);
                cx.notify();
            });
        // Chats read before this panel existed.
        let panel = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let _ = panel.update(cx, |panel, cx| panel.adopt_saved_chats(window, cx));
        });
        let (right_split, right_subscription) = split::split_state(1, cx);
        let (left_split, left_subscription) = split::split_state(0, cx);
        let usage_focus = cx.focus_handle();
        let usage_blur = cx.on_focus_out(&usage_focus, window, |this, _, _, cx| {
            this.dismiss_usage(cx);
        });
        let input_focus = input.read(cx).focus_handle(cx);
        let usage_input_blur = cx.on_focus_out(&input_focus, window, |this, _, window, cx| {
            if !this.usage_focus.contains_focused(window, cx) {
                this.dismiss_usage(cx);
            }
        });
        Self {
            image_cache: HashMap::new(),
            sign_in_inputs: HashMap::new(),
            focus: cx.focus_handle(),
            workspace,
            threads: Vec::new(),
            active: None,
            history: false,
            widened: None,
            splits: [right_split, left_split],
            docked_left: false,
            maximized: false,
            input,
            search,
            history_search,
            renaming: None,
            menu: None,
            usage_focus,
            list: cx
                .new(|cx| gpui_kit::component::message_scroller::MessageScrollerState::new(0, cx)),
            stream: Default::default(),
            stream_tick: None,
            queue_open: false,
            queue_heights: HashMap::new(),
            composer: Default::default(),
            commands: Default::default(),
            list_count: 0,
            expanded: HashSet::new(),
            error: None,
            subscriptions: [
                Some(submit),
                Some(input_observer),
                Some(search_observer),
                Some(history_search_observer),
                Some(settings),
                Some(runtime_observer),
                Some(right_subscription),
                Some(left_subscription),
                Some(usage_blur),
                Some(usage_input_blur),
                workspace_observer,
            ]
            .into_iter()
            .flatten()
            .collect(),
            notify_queued: false,
            approval_notice: None,
            history_tick: None,
        }
    }
    fn current(&self) -> Option<Entity<AgentThread>> {
        self.active
            .and_then(|index| self.threads.get(index))
            .cloned()
    }
    /// The agent `NewThreadWithLastAgent` starts: the last one used, else
    /// the default, else the first enabled one.
    fn last_agent(&self, cx: &App) -> Option<String> {
        let registry = nocterm_ai::AgentRegistry::new(&cx.settings().ai);
        let known = |id: &&String| registry.get(id).is_some();
        Runtime::global(cx)
            .read(cx)
            .favorites
            .last_agent
            .as_ref()
            .filter(known)
            .or(cx.settings().ai.default_agent.as_ref().filter(known))
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
        let text = self.input.read(cx).value().to_string();
        let replace = self
            .composer
            .edit
            .as_ref()
            .filter(|edit| edit.thread == thread.entity_id())
            .map(|edit| edit.id);
        let result = thread.update(cx, |thread, cx| thread.submit(text, replace, cx));
        match result {
            Ok(true) => {
                self.input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.finish_queue_edit(window, cx);
            }
            Ok(false) => return,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        }
        self.menu = None;
        cx.notify();
    }
    fn toggle_menu(&mut self, menu: MenuKind, cx: &mut Context<Self>) {
        self.menu = if self.menu.as_ref() == Some(&menu) {
            None
        } else {
            Some(menu)
        };
        cx.notify();
    }
    fn sync_approval_attention(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let attention = self.threads.iter().any(|thread| {
            !thread.read(cx).tools.is_empty() || !thread.read(cx).permissions.is_empty()
        });
        let needs_notice = attention
            && (self.current().is_none_or(|thread| {
                thread.read(cx).tools.is_empty() && thread.read(cx).permissions.is_empty()
            }) || self
                .workspace
                .upgrade()
                .is_none_or(|workspace| !workspace.read(cx).right_panel_is_open()));
        let _ = self.workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel_attention(attention, cx)
        });
        let requests = needs_notice.then(|| {
            self.threads
                .iter()
                .filter_map(|thread| {
                    let state = thread.read(cx);
                    (!state.tools.is_empty() || !state.permissions.is_empty())
                        .then_some((thread.entity_id(), state.approval_generation))
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
        let workspace = self.workspace.clone();
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
        let pending = |thread: &Entity<AgentThread>| {
            !thread.read(cx).permissions.is_empty() || !thread.read(cx).tools.is_empty()
        };
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
        self.maximized = maximized;
        cx.notify();
    }
    fn set_docked_left(&mut self, left: bool, cx: &mut Context<Self>) {
        self.docked_left = left;
        cx.notify();
    }
}
impl Render for AgentPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current();
        if self.history && !self.threads.is_empty() {
            if self.history_tick.is_none() {
                self.history_tick = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_secs(30))
                            .await;
                        if !this
                            .update(cx, |this, cx| {
                                if this.history {
                                    cx.notify();
                                }
                                this.history
                            })
                            .unwrap_or(false)
                        {
                            break;
                        }
                    }
                }));
            }
        } else {
            self.history_tick = None;
        }
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
                this.set_history(!this.history, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::StopGeneration, _, cx| {
                if let Some(thread) = this.current() {
                    thread.update(cx, |thread, cx| thread.stop(cx));
                }
            }))
            .on_action(cx.listener(|this, _: &crate::ToggleModelPicker, _, cx| {
                if let Some(thread) = this.current()
                    && let Some(option) =
                        thread.read(cx).state.config_options.iter().find(|option| {
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
                            .map(|thread| thread.read(cx).agent_id.clone());
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
                            .selected(self.history)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.set_history(!this.history, cx)),
                            ),
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
                            .icon(if self.maximized {
                                IconName::Minimize2
                            } else {
                                IconName::Maximize2
                            })
                            .tooltip(if self.maximized {
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
            Some(thread) if self.history => {
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
        if let Some(error) = &Runtime::global(cx).read(cx).favorites_error {
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

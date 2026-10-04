use crate::{
    runtime::Runtime,
    thread::{AgentThread, Attachment},
};
use gpui_kit::{
    Anchor, App, Context, Entity, EventEmitter, FocusHandle, Focusable, ListAlignment, ListState,
    SharedString, Subscription, Task, TestSupportExt as _, WeakEntity, Window,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
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
use nocterm_workspace::{Panel, RightPanel, RightPanelEvent, Workspace};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use widgets::HEADER_HEIGHT;

mod approvals;
mod attach;
mod chat;
mod composer;
mod entries;
mod history;
mod menu;
mod message_actions;
mod servers;
mod split;
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
    list: ListState,
    list_count: usize,
    expanded: HashSet<usize>,
    error: Option<String>,
    subscriptions: Vec<Subscription>,
    notify_queued: bool,
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
        let search_observer = cx.observe(&search, |_, _, cx| cx.notify());
        let history_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search chats…"));
        let history_search_observer = cx.observe(&history_search, |_, _, cx| cx.notify());
        let settings = cx.observe_global_in::<SettingsStore>(window, |this, window, cx| {
            let enabled = cx.ai_enabled();
            if !enabled {
                this.threads.clear();
                this.image_cache.clear();
                this.active = None;
                this.menu = None;
                this.sync_approval_attention(window, cx);
                this.input
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            let workspace = this.workspace.clone();
            window.defer(cx, move |window, cx| {
                let _ = workspace.update(cx, |workspace, cx| {
                    workspace.set_right_panel_available(enabled, window, cx)
                });
            });
            cx.notify();
        });
        let workspace_observer = workspace
            .upgrade()
            .map(|workspace| cx.observe(&workspace, |_, _, cx| cx.notify()));
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
            list: ListState::new(0, ListAlignment::Bottom, px(500.)),
            list_count: 0,
            expanded: HashSet::new(),
            error: None,
            subscriptions: [
                Some(submit),
                Some(search_observer),
                Some(history_search_observer),
                Some(settings),
                Some(runtime_observer),
                Some(right_subscription),
                Some(left_subscription),
                workspace_observer,
            ]
            .into_iter()
            .flatten()
            .collect(),
            notify_queued: false,
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
    fn new_thread(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.new_thread_connection(id, false, window, cx);
    }
    fn new_thread_connection(
        &mut self,
        id: String,
        fresh: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(launch) = nocterm_ai::AgentRegistry::new(&cx.settings().ai)
            .get(&id)
            .cloned()
        else {
            return;
        };
        Runtime::global(cx).update(cx, |runtime, cx| runtime.set_last_agent(&id, cx));
        let active = self
            .workspace
            .upgrade()
            .and_then(|workspace| workspace.read(cx).active_terminal(cx));
        // An empty chat left behind is not history.
        self.discard_drafts(None, cx);
        let thread = cx.new(|cx| AgentThread::new(id, self.workspace.clone(), cx));
        if let Some(id) = active {
            thread.update(cx, |thread, _| {
                thread.attachments.push(Attachment::Terminal(id))
            });
        }
        self.track(&thread, window, cx);
        self.connect_thread(&thread, launch, fresh, cx);
        self.threads.push(thread);
        self.active = Some(self.threads.len() - 1);
        self.menu = None;
        self.list.reset(0);
        self.list_count = 0;
        self.expanded.clear();
        self.image_cache.clear();
        window.focus(&self.input.read(cx).focus_handle(cx), cx);
        cx.notify();
    }
    /// Replaces the open chat, whose session ended, with the same chat in a
    /// new agent process.
    fn restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(old) = self.current() else {
            return;
        };
        let (agent, chat_id, name, pinned, state, attachments, last_prompt) = {
            let old = old.read(cx);
            (
                old.agent_id.clone(),
                old.chat_id.clone(),
                old.name.clone(),
                old.pinned,
                old.state.clone(),
                old.attachments.clone(),
                old.last_prompt.clone(),
            )
        };
        self.new_thread_connection(agent, true, window, cx);
        if let Some(thread) = self.current() {
            thread.update(cx, |thread, cx| {
                thread.chat_id = chat_id;
                thread.name = name;
                thread.pinned = pinned;
                thread.state = state;
                thread.attachments = attachments;
                thread.last_prompt = last_prompt;
                cx.notify();
            });
        }
        let old = old.entity_id();
        self.retain_threads(|thread, _| thread.entity_id() != old, cx);
    }
    /// Registers `thread` with the terminal bridge and connects its agent.
    fn connect_thread(
        &mut self,
        thread: &Entity<AgentThread>,
        launch: nocterm_ai::AgentLaunch,
        fresh: bool,
        cx: &mut Context<Self>,
    ) {
        let runtime = Runtime::global(cx);
        let registration = runtime.update(cx, |runtime, cx| runtime.register_bridge(thread, cx));
        match registration {
            Ok(registration) => {
                thread.update(cx, |thread, _| thread.registration = Some(registration))
            }
            Err(error) => thread.update(cx, |thread, cx| thread.fail(&error, cx)),
        };
        if thread.read(cx).registration.is_some() {
            runtime.update(cx, |runtime, cx| {
                runtime.connect(thread.clone(), launch, fresh, cx)
            });
        }
    }
    /// Shows the chats saved in earlier runs, oldest first, before the
    /// chats of this run.
    fn adopt_saved_chats(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !cx.ai_enabled() {
            return;
        }
        let Some(mut chats) =
            Runtime::global(cx).update(cx, |runtime, _| runtime.take_saved_chats())
        else {
            return;
        };
        if chats.is_empty() {
            return;
        }
        chats.reverse();
        let restored: Vec<_> = chats
            .into_iter()
            .map(|chat| {
                let thread = cx.new(|cx| AgentThread::restored(chat, self.workspace.clone(), cx));
                self.track(&thread, window, cx);
                thread
            })
            .collect();
        let count = restored.len();
        self.threads.splice(0..0, restored);
        self.active = self.active.map(|active| active + count);
        cx.notify();
    }
    /// Connects a chat restored from history when it is opened.
    fn wake(&mut self, thread: &Entity<AgentThread>, cx: &mut Context<Self>) {
        if !thread.read(cx).dormant {
            return;
        }
        thread.update(cx, |thread, _| thread.dormant = false);
        let agent = thread.read(cx).agent_id.clone();
        match nocterm_ai::AgentRegistry::new(&cx.settings().ai)
            .get(&agent)
            .cloned()
        {
            Some(launch) => self.connect_thread(thread, launch, false, cx),
            None => thread.update(cx, |thread, cx| {
                thread.fail(
                    &format!("The agent `{agent}` is no longer configured. Start a new chat."),
                    cx,
                )
            }),
        }
    }
    fn track(&mut self, thread: &Entity<AgentThread>, window: &mut Window, cx: &mut Context<Self>) {
        // Background sessions the chat opens belong to this window.
        let handle = window.window_handle();
        thread.update(cx, |thread, _| thread.window = Some(handle));
        self.subscriptions
            .push(cx.observe_in(thread, window, |this, _, window, cx| {
                let panel = cx.weak_entity();
                window.defer(cx, move |window, cx| {
                    let _ = panel.update(cx, |panel, cx| panel.sync_approval_attention(window, cx));
                });
                if this.notify_queued {
                    return;
                }
                this.notify_queued = true;
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(33))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.notify_queued = false;
                        cx.notify();
                    });
                })
                .detach();
            }));
    }
    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.current() else {
            return;
        };
        if thread.read(cx).generating
            || thread.read(cx).session.is_none()
            || thread.read(cx).ended()
        {
            return;
        }
        let text = self.input.read(cx).value().to_string();
        thread.update(cx, |thread, cx| thread.send(text, cx));
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
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
            self.active = self
                .threads
                .iter()
                .position(|thread| thread.entity_id() == owner.entity_id());
            self.list.reset(0);
            self.list_count = 0;
            self.expanded.clear();
            self.image_cache.clear();
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
            .bg(cx.theme().background)
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
                            .child(self.title(cx)),
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

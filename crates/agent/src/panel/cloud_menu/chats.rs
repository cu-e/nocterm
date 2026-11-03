//! Cloud chats in the Cloud tab: new chats on the person's own runners, the history of earlier
//! chats, and the chat of a Cloud agent that has just become ready.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use gpui_kit::{
    AnyElement, Context, SharedString, TestSupportExt as _, Window,
    component::{ActiveTheme as _, StyledExt as _, v_flex},
    div,
    prelude::*,
};
use nocterm_cloud_agents::{StartId, StartPhase};
use nocterm_cloud_agents_model::{
    ChatFuture, ChatHistoryEntry, ChatHost, ChatRunner, CloudChatError, OpenedChat,
};
use nocterm_cloud_agents_model::{CloudAgent, Placement};
use nocterm_hubs_model::HubId;

use super::{CloudMenu, MenuTab};
use crate::cloud_access::CloudAgentsAccess;
use crate::panel::{AgentPanel, widgets::menu_row};

/// What the Cloud tab remembers about chats: the runners and earlier chats last listed per
/// hub, the starts this window began, and the last failure to show.
#[derive(Default)]
pub(super) struct CloudChats {
    runners: BTreeMap<HubId, Vec<ChatRunner>>,
    history: BTreeMap<HubId, Vec<Arc<dyn ChatHistoryEntry>>>,
    started_here: BTreeSet<StartId>,
    notice: Option<String>,
}

/// What a failed attempt tells the person.
pub(in crate::panel) fn notice_text(error: &CloudChatError) -> String {
    if error.is_retryable() {
        format!("{error}. Try again.")
    } else {
        error.to_string()
    }
}

impl CloudMenu {
    /// Lists the runners and earlier chats of the shown hubs while the Cloud tab is selected.
    pub(super) fn refresh_chats(&mut self, cx: &mut Context<AgentPanel>) {
        let Some(access) = self.access.clone() else {
            return;
        };
        let Some(launcher) = access.chats().cloned() else {
            return;
        };
        if self.tab(cx) != MenuTab::Cloud {
            return;
        }
        for (hub, ..) in access.visible_hubs(cx) {
            let launcher = launcher.clone();
            cx.spawn(async move |panel, cx| {
                let runners = launcher.runners(&hub).await;
                let history = launcher.history(&hub).await;
                let _ = panel.update(cx, |panel, cx| {
                    let chats = &mut panel.cloud.chats;
                    match runners {
                        Ok(runners) => {
                            chats.runners.insert(hub.clone(), runners);
                        }
                        Err(CloudChatError::Unsupported) => {
                            chats.runners.remove(&hub);
                        }
                        // A listing nobody asked to see is not worth a warning; the rows stay
                        // as they were.
                        Err(_) => {}
                    }
                    if let Ok(history) = history {
                        chats.history.insert(hub, history);
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// The rows of the chat section: new chats per runner, then the history.
    pub(super) fn render_chats(
        &self,
        access: &CloudAgentsAccess,
        cx: &mut Context<AgentPanel>,
    ) -> Vec<AnyElement> {
        if access.chats().is_none() {
            return Vec::new();
        }
        let mut rows = Vec::new();
        for (hub, ..) in access.visible_hubs(cx) {
            let runners = self.chats.runners.get(&hub).cloned().unwrap_or_default();
            for runner in runners {
                let id = format!("cloud-chat-new-{}-{}", hub.address(), runner.label);
                let label = format!("New chat on {}", runner.label);
                let hub = hub.clone();
                rows.push(
                    menu_row(SharedString::from(id), label, cx)
                        .on_click(cx.listener(move |panel, _, window, cx| {
                            panel.cloud.create_chat(
                                &hub,
                                ChatHost::Runner(runner.clone()),
                                window,
                                cx,
                            );
                        }))
                        .into_any_element(),
                );
            }
            let history = self.chats.history.get(&hub).cloned().unwrap_or_default();
            if history.is_empty() {
                continue;
            }
            let mut block = v_flex().gap_1().child(
                div()
                    .id(SharedString::from(format!(
                        "cloud-chat-history-{}",
                        hub.address()
                    )))
                    .test_support()
                    .px_2()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child("History"),
            );
            for (index, entry) in history.into_iter().enumerate() {
                let id = format!("cloud-chat-entry-{}-{index}", hub.address());
                let label = entry.label().to_owned();
                block = block.child(menu_row(SharedString::from(id), label, cx).on_click(
                    cx.listener(move |panel, _, window, cx| {
                        panel.cloud.present_chat(entry.open(), None, window, cx);
                    }),
                ));
            }
            rows.push(block.into_any_element());
        }
        if let Some(notice) = &self.chats.notice {
            rows.push(
                div()
                    .id("cloud-chat-notice")
                    .test_support()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(cx.theme().warning)
                    .child(notice.clone())
                    .into_any_element(),
            );
        }
        rows
    }

    fn create_chat(
        &mut self,
        hub: &HubId,
        host: ChatHost,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) {
        let Some(launcher) = self.access.as_ref().and_then(|a| a.chats().cloned()) else {
            return;
        };
        self.present_chat(launcher.create(hub, host), None, window, cx);
    }

    /// A row was chosen: "chat with this agent". A seat the person already has opens its chat
    /// at once (waking it when it sleeps); otherwise the agent is started and its chat opens
    /// when it is ready.
    pub(super) fn activate_agent(
        &mut self,
        hub: &HubId,
        agent: &CloudAgent,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) {
        let Some(access) = self.access.clone() else {
            return;
        };
        let direct = self.seat_of(hub, agent, cx)
            && !access
                .service()
                .read(cx)
                .starts()
                .any(|start| start.hub() == hub && *start.agent() == agent.id);
        let launcher = access.chats().cloned();
        let model = agent.primary_model().map(str::to_owned);
        let (Some(launcher), Some(model), true) = (launcher, model, direct) else {
            self.start_agent(hub, agent, window, cx);
            return;
        };
        self.chats.notice = None;
        let host = ChatHost::Agent {
            agent: agent.id.clone(),
            model,
        };
        let future = launcher.create(hub, host);
        let (hub, agent) = (hub.clone(), agent.clone());
        cx.spawn_in(window, async move |panel, cx| {
            let result = future.await;
            let _ = panel.update_in(cx, |panel, window, cx| match result {
                // The seat is not usable as it is (it never finished enrolling): starting the
                // agent resumes and completes it.
                Err(CloudChatError::AgentNotStarted) => {
                    panel.cloud.start_agent(&hub, &agent, window, cx);
                    cx.notify();
                }
                other => panel.finish_cloud_chat(other, None, window, cx),
            });
        })
        .detach();
        cx.notify();
    }

    /// The popover opened again: what it showed before (failures, questions) is not shown
    /// twice.
    pub(in crate::panel) fn reopened(&mut self, cx: &mut Context<AgentPanel>) {
        self.forget_transient();
        self.dismiss_failed_starts(cx);
    }

    /// Forgets the notice and the removal question of an earlier visit.
    pub(in crate::panel) fn forget_transient(&mut self) {
        self.chats.notice = None;
        self.removing = None;
    }

    /// Remembers that this window pressed Start; opens the chat at once when the start had
    /// already finished.
    pub(super) fn began_start(
        &mut self,
        id: StartId,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) {
        self.chats.started_here.insert(id);
        if self.start_is_ready(id, cx) {
            self.open_started_chat(id, window, cx);
        }
    }

    /// A start became ready: its chat opens in the window that pressed Start.
    pub(super) fn start_ready(
        &mut self,
        id: StartId,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) {
        if self.chats.started_here.contains(&id) {
            self.open_started_chat(id, window, cx);
        }
    }

    fn start_is_ready(&self, id: StartId, cx: &gpui_kit::App) -> bool {
        self.access.as_ref().is_some_and(|access| {
            access
                .service()
                .read(cx)
                .starts()
                .any(|start| start.id() == id && matches!(start.phase(), StartPhase::Ready(_)))
        })
    }

    /// Creates the chat of the agent a ready start belongs to and presents it.
    pub(super) fn open_started_chat(
        &mut self,
        id: StartId,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) {
        let Some(access) = self.access.clone() else {
            return;
        };
        let Some(launcher) = access.chats().cloned() else {
            return;
        };
        let service = access.service().read(cx);
        let target: Option<(HubId, ChatHost)> = service
            .starts()
            .find(|start| start.id() == id)
            .and_then(|start| {
                let agent = service
                    .agents(start.hub())
                    .iter()
                    .find(|agent| agent.id == *start.agent())?;
                let model = agent.primary_model()?.to_owned();
                let host = match (agent.placement(), start.phase()) {
                    (Placement::OwnRunner, StartPhase::Ready(started)) => {
                        let id: [u8; 16] = started.runner.as_slice().try_into().ok()?;
                        let label = self
                            .chats
                            .runners
                            .get(start.hub())
                            .and_then(|runners| runners.iter().find(|runner| runner.id == id))
                            .map_or_else(
                                || "your runner".to_owned(),
                                |runner| runner.label.clone(),
                            );
                        ChatHost::OwnAgent {
                            runner: ChatRunner { id, label },
                            agent: start.agent().clone(),
                            model,
                        }
                    }
                    _ => ChatHost::Agent {
                        agent: start.agent().clone(),
                        model,
                    },
                };
                Some((start.hub().clone(), host))
            });
        let Some((hub, host)) = target else {
            self.chats.notice = Some("The agent has no model to chat with.".to_owned());
            cx.notify();
            return;
        };
        self.chats.started_here.remove(&id);
        let future = launcher.create(&hub, host);
        self.present_chat(future, Some(id), window, cx);
    }

    /// Waits for `future`, then shows the chat it opened. `start` is dismissed once its chat
    /// is open.
    fn present_chat(
        &mut self,
        future: ChatFuture<OpenedChat>,
        start: Option<StartId>,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) {
        self.chats.notice = None;
        cx.spawn_in(window, async move |panel, cx| {
            let result = future.await;
            let _ = panel.update_in(cx, |panel, window, cx| {
                panel.finish_cloud_chat(result, start, window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// The chat a start belonged to is open: its start is dismissed.
    pub(in crate::panel) fn chat_opened(
        &self,
        start: Option<StartId>,
        cx: &mut Context<AgentPanel>,
    ) {
        if let (Some(id), Some(access)) = (start, &self.access) {
            access
                .service()
                .update(cx, |service, cx| service.dismiss(id, cx));
        }
    }

    /// Shows `notice` in the tab until the next chat opens.
    pub(super) fn show_notice(&mut self, notice: String) {
        self.chats.notice = Some(notice);
    }

    /// Opening the chat failed: the person sees `notice`, and the start stays pending.
    pub(in crate::panel) fn chat_failed(&mut self, notice: String, start: Option<StartId>) {
        self.show_notice(notice);
        if let Some(id) = start {
            self.chats.started_here.insert(id);
        }
    }
}

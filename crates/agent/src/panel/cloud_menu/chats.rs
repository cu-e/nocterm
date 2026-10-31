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
    AgentId, ChatFuture, ChatHistoryEntry, ChatHost, ChatRunner, CloudChatError, OpenedChat,
};
use nocterm_hubs_model::HubId;

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
fn notice_text(error: &CloudChatError) -> String {
    if error.is_retryable() {
        format!("{error}. Try again.")
    } else {
        error.to_string()
    }
}

impl AgentPanel {
    /// Lists the runners and earlier chats of the shown hubs while the Cloud tab is selected.
    pub(super) fn refresh_cloud_chats(&mut self, cx: &mut Context<Self>) {
        let Some(access) = self.cloud.access.clone() else {
            return;
        };
        let Some(launcher) = access.chats().cloned() else {
            return;
        };
        if self.cloud.tab(cx) != super::MenuTab::Cloud {
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
                        Err(error) => chats.notice = Some(notice_text(&error)),
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
    pub(super) fn render_cloud_chats(
        &mut self,
        access: &CloudAgentsAccess,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if access.chats().is_none() {
            return Vec::new();
        }
        let mut rows = Vec::new();
        for (hub, ..) in access.visible_hubs(cx) {
            let runners = self
                .cloud
                .chats
                .runners
                .get(&hub)
                .cloned()
                .unwrap_or_default();
            for runner in runners {
                let id = format!("cloud-chat-new-{}-{}", hub.address(), runner.label);
                let label = format!("New Cloud chat · {}", runner.label);
                let hub = hub.clone();
                rows.push(
                    menu_row(SharedString::from(id), label, cx)
                        .on_click(cx.listener(move |panel, _, window, cx| {
                            panel.create_cloud_chat(
                                &hub,
                                ChatHost::Runner(runner.clone()),
                                window,
                                cx,
                            );
                        }))
                        .into_any_element(),
                );
            }
            let history = self
                .cloud
                .chats
                .history
                .get(&hub)
                .cloned()
                .unwrap_or_default();
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
                        panel.present_cloud_chat(entry.open(), None, window, cx);
                    }),
                ));
            }
            rows.push(block.into_any_element());
        }
        if let Some(notice) = &self.cloud.chats.notice {
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

    fn create_cloud_chat(
        &mut self,
        hub: &HubId,
        host: ChatHost,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(launcher) = self.cloud.access.as_ref().and_then(|a| a.chats().cloned()) else {
            return;
        };
        self.present_cloud_chat(launcher.create(hub, host), None, window, cx);
    }

    /// Remembers that this window pressed Start; opens the chat at once when the start had
    /// already finished.
    pub(super) fn began_cloud_start(
        &mut self,
        id: StartId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cloud.chats.started_here.insert(id);
        if self.start_is_ready(id, cx) {
            self.open_started_chat(id, window, cx);
        }
    }

    /// A start became ready: its chat opens in the window that pressed Start.
    pub(super) fn cloud_start_ready(
        &mut self,
        id: StartId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cloud.chats.started_here.contains(&id) {
            self.open_started_chat(id, window, cx);
        }
    }

    fn start_is_ready(&self, id: StartId, cx: &gpui_kit::App) -> bool {
        self.cloud.access.as_ref().is_some_and(|access| {
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
        cx: &mut Context<Self>,
    ) {
        let Some(access) = self.cloud.access.clone() else {
            return;
        };
        let Some(launcher) = access.chats().cloned() else {
            return;
        };
        let service = access.service().read(cx);
        let target: Option<(HubId, AgentId, String)> = service
            .starts()
            .find(|start| start.id() == id)
            .and_then(|start| {
                let agent = service
                    .agents(start.hub())
                    .iter()
                    .find(|agent| agent.id == *start.agent())?;
                let model = agent.primary_model()?.to_owned();
                Some((start.hub().clone(), start.agent().clone(), model))
            });
        let Some((hub, agent, model)) = target else {
            self.cloud.chats.notice = Some("The agent has no model to chat with.".to_owned());
            cx.notify();
            return;
        };
        self.cloud.chats.started_here.remove(&id);
        let future = launcher.create(&hub, ChatHost::Agent { agent, model });
        self.present_cloud_chat(future, Some(id), window, cx);
    }

    /// Waits for `future`, then shows the chat it opened. `start` is dismissed once its chat
    /// is open.
    fn present_cloud_chat(
        &mut self,
        future: ChatFuture<OpenedChat>,
        start: Option<StartId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cloud.chats.notice = None;
        cx.spawn_in(window, async move |panel, cx| {
            let result = future.await;
            let _ = panel.update_in(cx, |panel, window, cx| {
                panel.finish_cloud_chat(result, start, window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_cloud_chat(
        &mut self,
        result: Result<OpenedChat, CloudChatError>,
        start: Option<StartId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let outcome = result
            .map_err(|error| notice_text(&error))
            .and_then(|opened| {
                let source = self
                    .host
                    .cloud_chats()
                    .open(opened.source, cx)
                    .map_err(|error| error.to_string())?;
                self.open_conversation(source, SharedString::from(opened.title), window, cx)
                    .map_err(|error| error.to_string())
            });
        match outcome {
            Ok(()) => {
                if let (Some(id), Some(access)) = (start, &self.cloud.access) {
                    access
                        .service()
                        .update(cx, |service, cx| service.dismiss(id, cx));
                }
                self.menus.reset();
            }
            Err(notice) => {
                self.cloud.chats.notice = Some(notice);
                if let Some(id) = start {
                    self.cloud.chats.started_here.insert(id);
                }
            }
        }
        cx.notify();
    }
}

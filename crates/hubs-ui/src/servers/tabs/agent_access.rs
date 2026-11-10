//! The Agent access tab of a server: the account's agent key, and the servers connected to the
//! agents with what agents last found there.
//!
//! A server is connected from the connection editor of a saved server ("Agent access"), which
//! writes the key's line to its `authorized_keys`. Here the person checks a server again,
//! disconnects it (agents stop using it at once; the line stays until it is removed on the
//! server) and replaces the key (every server then needs its line written again).

use gpui_kit::{
    ClickEvent, ClipboardItem, Context, Entity, Subscription, Task, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::DialogFooter,
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_hub_agents::HubAgents;
use nocterm_hub_agents_model::{AgentKey, HubServer, ServerId};
use nocterm_hubs_model::HubId;

use crate::layout::{field, note, problem};
use crate::messages::agents_problem;
use crate::servers::item;

/// The agent key and servers of the server that is shown.
pub(in crate::servers) struct AgentAccessTab {
    service: Entity<HubAgents>,
    hub: Option<HubId>,
    key: Option<AgentKey>,
    problem: Option<String>,
    working: Option<Task<()>>,
    _service: Subscription,
}

impl AgentAccessTab {
    pub(in crate::servers) fn new(service: Entity<HubAgents>, cx: &mut Context<Self>) -> Self {
        Self {
            _service: cx.observe(&service, |_, _, cx| cx.notify()),
            service,
            hub: None,
            key: None,
            problem: None,
            working: None,
        }
    }

    /// Shows the agent key and servers of `hub`, read again when another server was shown.
    pub(in crate::servers) fn show(&mut self, hub: &HubId, cx: &mut Context<Self>) {
        if self.hub.as_ref() == Some(hub) {
            return;
        }
        self.hub = Some(hub.clone());
        self.key = None;
        self.problem = None;
        self.service.update(cx, |service, cx| {
            service.track(hub, cx);
            service.refresh(hub, cx);
        });
        let reading = self.service.read(cx).server_port().agent_key(hub);
        self.key_from(reading, cx);
    }

    pub(in crate::servers) fn hide(&mut self) {
        self.hub = None;
        self.working = None;
    }

    fn key_from(
        &mut self,
        reading: nocterm_hub_agents_model::AgentsFuture<AgentKey>,
        cx: &mut Context<Self>,
    ) {
        self.working = Some(cx.spawn(async move |this, cx| {
            let result = reading.await;
            this.update(cx, |tab, cx| {
                match result {
                    Ok(key) => tab.key = Some(key),
                    Err(error) => tab.problem = Some(agents_problem(&error)),
                }
                if let Some(hub) = tab.hub.clone() {
                    tab.service
                        .update(cx, |service, cx| service.refresh(&hub, cx));
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn rotate(&mut self, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let rotating = self.service.read(cx).server_port().rotate_agent_key(&hub);
        self.key_from(rotating, cx);
    }

    /// Runs a call on one server, then lists the servers again.
    fn on_server<T: Send + 'static>(
        &mut self,
        call: nocterm_hub_agents_model::AgentsFuture<T>,
        cx: &mut Context<Self>,
    ) {
        self.problem = None;
        self.working = Some(cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |tab, cx| {
                if let Err(error) = result {
                    tab.problem = Some(agents_problem(&error));
                }
                if let Some(hub) = tab.hub.clone() {
                    tab.service
                        .update(cx, |service, cx| service.refresh(&hub, cx));
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn check(&mut self, server: &ServerId, cx: &mut Context<Self>) {
        if let Some(hub) = self.hub.clone() {
            let checking = self
                .service
                .read(cx)
                .server_port()
                .check_server(&hub, server);
            self.on_server(checking, cx);
        }
    }

    fn disconnect(&mut self, server: &ServerId, cx: &mut Context<Self>) {
        if let Some(hub) = self.hub.clone() {
            let removing = self
                .service
                .read(cx)
                .server_port()
                .remove_server(&hub, server);
            self.on_server(removing, cx);
        }
    }

    /// Asks before replacing the key: every server needs its line written again.
    fn confirm_rotate(view: Entity<Self>, window: &mut Window, cx: &mut gpui_kit::App) {
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            let confirm = Button::new("agent-key-rotate-confirm")
                .danger()
                .label("Replace the key")
                .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut gpui_kit::App| {
                    window.close_dialog(cx);
                    view.update(cx, |tab, cx| tab.rotate(cx));
                });
            let keep = Button::new("agent-key-rotate-keep")
                .primary()
                .label("Keep it")
                .on_click(|_: &ClickEvent, window: &mut Window, cx: &mut gpui_kit::App| {
                    window.close_dialog(cx);
                });
            alert
                .title("Replace the agent key?")
                .description(
                    "Agents stop signing in to every connected server until its line is written again: turn Agent access off and on in each server's editor.",
                )
                .footer(DialogFooter::new().child(keep).child(confirm))
        });
    }

    fn render_key(&self, key: &str, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(agent_key) = &self.key else {
            return note("Reading the agent key…", cx).into_any_element();
        };
        let line = agent_key.authorized_line.clone();
        field(
            "Agent key",
            v_flex()
                .id(item("agent-key", key))
                .test_support()
                .gap_1()
                .child(
                    div()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_sm()
                        .child(agent_key.fingerprint.clone()),
                )
                .child(note(
                    "Agents sign in to your servers with this key. Its line goes into authorized_keys with port, agent and X11 forwarding off.",
                    cx,
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(item("agent-key-copy", key))
                                .small()
                                .label("Copy line")
                                .on_click(move |_: &ClickEvent, _, cx: &mut gpui_kit::App| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(line.clone()));
                                }),
                        )
                        .child(
                            Button::new(item("agent-key-rotate", key))
                                .small()
                                .ghost()
                                .label("Replace key")
                                .on_click(cx.listener(|_, _: &ClickEvent, window, cx| {
                                    Self::confirm_rotate(cx.entity(), window, cx);
                                })),
                        ),
                ),
        )
        .into_any_element()
    }

    fn render_server(server: &HubServer, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let (check, remove) = (server.id.clone(), server.id.clone());
        let status = if server.status_message.is_empty() {
            server.status.label().to_owned()
        } else {
            format!("{} — {}", server.status.label(), server.status_message)
        };
        h_flex()
            .id(item("agent-server", server.id.as_str()))
            .test_support()
            .gap_2()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().font_semibold().child(server.name.clone()))
                    .child(note(
                        format!("{}@{}:{} · {status}", server.user, server.host, server.port),
                        cx,
                    )),
            )
            .child(
                Button::new(item("agent-server-check", server.id.as_str()))
                    .small()
                    .ghost()
                    .label("Check")
                    .on_click(cx.listener(move |tab, _: &ClickEvent, _, cx| tab.check(&check, cx))),
            )
            .child(
                Button::new(item("agent-server-disconnect", server.id.as_str()))
                    .small()
                    .ghost()
                    .label("Disconnect")
                    .on_click(cx.listener(move |tab, _: &ClickEvent, _, cx| {
                        tab.disconnect(&remove, cx);
                    })),
            )
            .into_any_element()
    }
}

impl Render for AgentAccessTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(hub) = self.hub.clone() else {
            return div().into_any_element();
        };
        let key = hub.to_string();
        let servers = self.service.read(cx).servers(&hub).to_vec();
        let rows: Vec<_> = servers
            .iter()
            .map(|server| Self::render_server(server, cx))
            .collect();
        v_flex()
            .id(item("agent-access-tab", &key))
            .test_support()
            .gap_3()
            .children(self.problem.clone().map(|text| problem(text, cx)))
            .child(self.render_key(&key, cx))
            .child(div().font_semibold().child("Servers agents may use"))
            .when(rows.is_empty(), |tab| {
                tab.child(note(
                    "None yet. Open a saved server in this window's connection editor and turn on Agent access.",
                    cx,
                ))
            })
            .children(rows)
            .into_any_element()
    }
}

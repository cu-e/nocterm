//! The Agent access (SSH) tab of a server: the CA that signs the certificates cloud agents use
//! to reach the user's servers, the servers that trust it, and the chat accesses agents hold.
//!
//! The tab reads through the service whenever it is shown for a server and again after each
//! change. It never decides what a server trusts: allowing and denying go to the service, which
//! edits `authorized_keys` through a session the user already opened, and a server that could
//! not be reached is reported as pending, not as removed.

use std::collections::BTreeSet;

use gpui_kit::{
    App, AppContext as _, ClickEvent, ClipboardItem, Context, Entity, Subscription, Task,
    TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        checkbox::Checkbox,
        dialog::DialogFooter,
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_hubs::Hubs;
use nocterm_hubs_model::{
    AgentAccessError, AgentAccessFuture, AgentAccessGrant, AgentTrustOverview, HostChoice, HubId,
    ServerTrust, TrustedServer,
};

use super::account_keys::{AccountKeys, AccountKeysEvent};
use super::devices::utc;
use crate::layout::{note, problem};
use crate::messages::agent_problem;
use crate::servers::item;

/// What the tab shows of the account.
enum Load {
    Loading,
    Ready {
        overview: AgentTrustOverview,
        grants: Vec<AgentAccessGrant>,
    },
    Failed(AgentAccessError),
}

/// The agent CA, the servers that trust it and the chat accesses of the server that is shown.
pub(in crate::servers) struct AgentAccess {
    hubs: Entity<Hubs>,
    hub: Option<HubId>,
    load: Load,
    /// Connections ticked for the bulk denial.
    selected: BTreeSet<String>,
    /// The connections ticked in the open "Allow on servers…" list.
    picking: Option<BTreeSet<String>>,
    /// The outcome of the last change, as a sentence.
    notice: Option<Result<String, String>>,
    loading: Option<Task<()>>,
    working: Option<Task<()>>,
    /// The lock of the account keys the records are encrypted under.
    keys: Entity<AccountKeys>,
    _keys_changed: Subscription,
}

impl AgentAccess {
    pub(in crate::servers) fn new(hubs: Entity<Hubs>, cx: &mut Context<Self>) -> Self {
        let keys = cx.new(|_| AccountKeys::new(hubs.clone()));
        let _keys_changed = cx.subscribe(&keys, |tab, _, event: &AccountKeysEvent, cx| {
            let AccountKeysEvent::Changed = event;
            tab.reload(cx);
        });
        Self {
            hubs,
            hub: None,
            load: Load::Loading,
            selected: BTreeSet::new(),
            picking: None,
            notice: None,
            loading: None,
            working: None,
            keys,
            _keys_changed,
        }
    }

    /// Shows `hub`, reading its records when another server was shown before.
    pub(in crate::servers) fn show(&mut self, hub: &HubId, cx: &mut Context<Self>) {
        if self.hub.as_ref() == Some(hub) {
            return;
        }
        self.hub = Some(hub.clone());
        self.keys.update(cx, |keys, cx| keys.show(hub, cx));
        self.selected.clear();
        self.picking = None;
        self.notice = None;
        self.reload(cx);
    }

    /// Forgets what was read: the next [`AgentAccess::show`] reads again.
    pub(in crate::servers) fn hide(&mut self) {
        self.hub = None;
        self.loading = None;
        self.working = None;
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        self.load = Load::Loading;
        let hubs = self.hubs.read(cx);
        let (overview, grants) = (hubs.agent_overview(&hub), hubs.agent_grants(&hub));
        self.loading = Some(cx.spawn(async move |this, cx| {
            let result = async { Ok((overview.await?, grants.await?)) }.await;
            this.update(cx, |tab, cx| {
                if tab.hub.as_ref() != Some(&hub) {
                    return;
                }
                tab.load = match result {
                    Ok((overview, grants)) => {
                        let known: BTreeSet<_> = overview
                            .servers
                            .iter()
                            .map(|server| server.connection.clone())
                            .collect();
                        tab.selected.retain(|connection| known.contains(connection));
                        Load::Ready { overview, grants }
                    }
                    Err(error) => Load::Failed(error),
                };
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Runs `change`, reports its outcome and reads the records again.
    fn change<T: 'static>(
        &mut self,
        change: AgentAccessFuture<T>,
        outcome: impl FnOnce(T) -> String + 'static,
        cx: &mut Context<Self>,
    ) {
        self.working = Some(cx.spawn(async move |this, cx| {
            let result = change.await;
            this.update(cx, |tab, cx| {
                tab.notice = Some(match result {
                    Ok(done) => Ok(outcome(done)),
                    Err(error) => Err(agent_problem(&error)),
                });
                tab.reload(cx);
            })
            .ok();
        }));
    }

    fn deny(&mut self, connection: &str, cx: &mut Context<Self>) {
        self.deny_all(vec![connection.to_owned()], cx);
    }

    /// Denies each connection in turn; the outcome names the ones that wait for a connection.
    fn deny_all(&mut self, connections: Vec<String>, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let hubs = self.hubs.clone();
        let denying: Vec<_> = connections
            .into_iter()
            .map(|connection| {
                let future = hubs.read(cx).deny_agent_host(&hub, &connection);
                (connection, future)
            })
            .collect();
        self.selected.clear();
        self.working = Some(cx.spawn(async move |this, cx| {
            let (mut pending, mut failure) = (Vec::new(), None);
            for (connection, future) in denying {
                match future.await {
                    Ok(ServerTrust::RemovalPending) => pending.push(connection),
                    Ok(_) => {}
                    Err(error) => failure = Some(agent_problem(&error)),
                }
            }
            this.update(cx, |tab, cx| {
                tab.notice = Some(match (failure, pending.is_empty()) {
                    (Some(problem), _) => Err(problem),
                    (None, true) => Ok("Access denied.".to_owned()),
                    (None, false) => Ok(format!(
                        "Access is denied. {} could not be reached and loses the CA line the next time you connect to it.",
                        pending.join(", ")
                    )),
                });
                tab.reload(cx);
            })
            .ok();
        }));
        cx.notify();
    }

    fn allow_picked(&mut self, cx: &mut Context<Self>) {
        let (Some(hub), Some(picked)) = (self.hub.clone(), self.picking.take()) else {
            return;
        };
        let hubs = self.hubs.clone();
        let allowing: Vec<_> = picked
            .into_iter()
            .map(|connection| {
                let future = hubs.read(cx).allow_agent_host(&hub, &connection);
                (connection, future)
            })
            .collect();
        self.working = Some(cx.spawn(async move |this, cx| {
            let (mut allowed, mut failure) = (0usize, None);
            for (_, future) in allowing {
                match future.await {
                    Ok(()) => allowed += 1,
                    Err(error) => failure = Some(agent_problem(&error)),
                }
            }
            this.update(cx, |tab, cx| {
                tab.notice = Some(match failure {
                    Some(problem) => Err(problem),
                    None => Ok(format!("Agents can sign in to {allowed} server(s).")),
                });
                tab.reload(cx);
            })
            .ok();
        }));
        cx.notify();
    }

    fn revoke(&mut self, grant_id: &str, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let revoking = self.hubs.read(cx).revoke_agent_grant(&hub, grant_id);
        self.change(revoking, |()| "Access revoked.".to_owned(), cx);
    }

    fn reissue(&mut self, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let reissuing = self.hubs.read(cx).reissue_agent_ca(&hub);
        self.change(
            reissuing,
            |ca| {
                format!(
                    "A new CA {} was issued. Servers that could not be reached trust the old one until you connect to them.",
                    ca.fingerprint
                )
            },
            cx,
        );
    }

    fn copy_line(&mut self, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let line = self.hubs.read(cx).agent_ca_line(&hub);
        self.working = Some(cx.spawn(async move |this, cx| {
            let result = line.await;
            this.update(cx, |tab, cx| {
                tab.notice = Some(match result {
                    Ok(line) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(line));
                        Ok(
                            "The CA line is copied. Add it to the server's authorized_keys."
                                .to_owned(),
                        )
                    }
                    Err(error) => Err(agent_problem(&error)),
                });
                cx.notify();
            })
            .ok();
        }));
    }

    fn toggle(set: &mut BTreeSet<String>, connection: &str, on: bool) {
        if on {
            set.insert(connection.to_owned());
        } else {
            set.remove(connection);
        }
    }

    /// Asks before replacing the CA: every agent certificate of the old one stops working.
    fn confirm_reissue(view: Entity<Self>, window: &mut Window, cx: &mut App) {
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            let confirm = Button::new("agent-reissue-confirm")
                .danger()
                .label("Reissue")
                .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    window.close_dialog(cx);
                    view.update(cx, |tab, cx| tab.reissue(cx));
                });
            let keep = Button::new("agent-reissue-keep")
                .primary()
                .label("Keep it")
                .on_click(|_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    window.close_dialog(cx);
                });
            alert
                .title("Reissue the CA?")
                .description(
                    "Certificates signed by the old CA stop working. Servers that can be reached now move to the new one; the others follow when you connect to them.",
                )
                .on_ok(|_, _, _| true)
                .footer(DialogFooter::new().child(confirm).child(keep))
        });
    }
}

impl Render for AgentAccess {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.load {
            Load::Loading => div()
                .id("agent-access-loading")
                .test_support()
                .child(note("Reading the agent access of your account…", cx))
                .into_any_element(),
            Load::Failed(error) => self.failed(error, cx),
            Load::Ready { overview, grants } => v_flex()
                .gap_4()
                .child(self.authority(overview, cx))
                .child(self.servers(overview, cx))
                .child(self.accesses(grants, cx))
                .into_any_element(),
        };
        let notice = self.notice.as_ref().map(|notice| match notice {
            Ok(text) => div()
                .id("agent-access-notice")
                .test_support()
                .child(note(text.clone(), cx))
                .into_any_element(),
            Err(text) => div()
                .id("agent-access-error")
                .test_support()
                .child(problem(text.clone(), cx))
                .into_any_element(),
        });
        v_flex()
            .id("agent-access-tab")
            .test_support()
            .gap_3()
            .child(note(
                "Cloud agents sign in to your servers with short-lived certificates from a CA only your devices hold. A server trusts the CA once its authorized_keys lists it.",
                cx,
            ))
            .children(notice)
            .child(self.keys.clone())
            .child(body)
    }
}

impl AgentAccess {
    fn failed(&self, error: &AgentAccessError, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let view = cx.entity();
        v_flex()
            .id("agent-access-failed")
            .test_support()
            .gap_2()
            .child(problem(agent_problem(error), cx))
            .child(
                h_flex().child(
                    Button::new("agent-access-retry")
                        .label("Try again")
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view.update(cx, |tab, cx| tab.reload(cx));
                        }),
                ),
            )
            .into_any_element()
    }

    /// The CA with the ways to copy or replace it.
    fn authority(
        &self,
        overview: &AgentTrustOverview,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let view = cx.entity();
        let (copy, reissue) = (view.clone(), view);
        let theme = cx.theme();
        let summary = match &overview.ca {
            Some(ca) => format!("{} · created {}", ca.fingerprint, utc(ca.created_at_ms)),
            None => "No CA yet. It is created when you allow the first server.".to_owned(),
        };
        let has_ca = overview.ca.is_some();
        h_flex()
            .id("agent-access-ca")
            .test_support()
            .gap_3()
            .items_center()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(div().font_semibold().child("Agent CA"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(summary),
                    ),
            )
            .child(
                Button::new("agent-copy-line")
                    .small()
                    .label("Copy CA line")
                    .disabled(!has_ca)
                    .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        copy.update(cx, |tab, cx| tab.copy_line(cx));
                    }),
            )
            .child(
                Button::new("agent-reissue")
                    .small()
                    .danger()
                    .label("Reissue CA…")
                    .disabled(!has_ca)
                    .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        Self::confirm_reissue(reissue.clone(), window, cx);
                    }),
            )
            .into_any_element()
    }

    /// The registry of servers, with the bulk denial and the list to allow more.
    fn servers(
        &self,
        overview: &AgentTrustOverview,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let view = cx.entity();
        let (allow, deny) = (view.clone(), view.clone());
        let shown: Vec<_> = overview
            .servers
            .iter()
            .filter(|server| server.state != ServerTrust::Removed)
            .collect();
        let ticked = self.selected.len();
        let trusted: BTreeSet<&str> = shown
            .iter()
            .map(|server| server.connection.as_str())
            .collect();
        let candidates: Vec<HostChoice> = self
            .hubs
            .read(cx)
            .agent_candidates()
            .into_iter()
            .filter(|choice| !trusted.contains(choice.connection.as_str()))
            .collect();
        let list = if shown.is_empty() {
            div()
                .id("agent-servers-empty")
                .test_support()
                .child(note("No server trusts the CA yet.", cx))
                .into_any_element()
        } else {
            v_flex()
                .gap_2()
                .children(shown.iter().map(|server| self.server_row(server, cx)))
                .into_any_element()
        };
        v_flex()
            .id("agent-servers")
            .test_support()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .font_semibold()
                            .child("Servers that trust the CA"),
                    )
                    .child(
                        Button::new("agent-deny-selected")
                            .small()
                            .danger()
                            .label("Deny on selected")
                            .disabled(ticked == 0)
                            .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                deny.update(cx, |tab, cx| {
                                    let chosen = tab.selected.iter().cloned().collect();
                                    tab.deny_all(chosen, cx);
                                });
                            }),
                    )
                    .child(
                        Button::new("agent-allow")
                            .small()
                            .primary()
                            .label("Allow on servers…")
                            .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                allow.update(cx, |tab, cx| {
                                    tab.picking = match tab.picking {
                                        Some(_) => None,
                                        None => Some(BTreeSet::new()),
                                    };
                                    cx.notify();
                                });
                            }),
                    ),
            )
            .when(self.picking.is_some(), |column| {
                column.child(self.picker(&candidates, cx))
            })
            .child(list)
            .into_any_element()
    }

    /// The connected servers that do not trust the CA yet, to tick and allow.
    fn picker(&self, candidates: &[HostChoice], cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        let picked = self.picking.clone().unwrap_or_default();
        let view = cx.entity();
        let confirm = Button::new("agent-allow-confirm")
            .small()
            .primary()
            .label("Allow")
            .disabled(picked.is_empty())
            .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                view.update(cx, |tab, cx| tab.allow_picked(cx));
            });
        let rows = candidates.iter().map(|choice| {
            let connection = choice.connection.clone();
            Checkbox::new(item("agent-pick", &choice.connection))
                .label(format!("{}@{} ({})", choice.user, choice.host, connection))
                .checked(picked.contains(&choice.connection))
                .on_click(cx.listener(move |tab, on: &bool, _, cx| {
                    if let Some(set) = tab.picking.as_mut() {
                        Self::toggle(set, &connection, *on);
                    }
                    cx.notify();
                }))
        });
        v_flex()
            .id("agent-picker")
            .test_support()
            .gap_2()
            .p_3()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .child(note(
                "Servers you are connected to now. The CA line is added to the user's authorized_keys over that session.",
                cx,
            ))
            .when(candidates.is_empty(), |column| {
                column.child(note(
                    "Every connected server trusts the CA already, or none is connected.",
                    cx,
                ))
            })
            .children(rows)
            .child(h_flex().child(confirm))
            .into_any_element()
    }

    fn server_row(&self, server: &TrustedServer, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        let (view, connection) = (cx.entity(), server.connection.clone());
        let (slug, state, colour) = match server.state {
            ServerTrust::Trusted => ("trusted", "Trusted", theme.success),
            ServerTrust::RemovalPending => (
                "pending",
                "Removal pending: connect to finish",
                theme.warning,
            ),
            ServerTrust::Removed => ("removed", "Removed", theme.muted_foreground),
            ServerTrust::Drifted => ("drifted", "Drifted: the CA line is missing", theme.danger),
        };
        let verified = server.last_verified_at_ms.map_or_else(
            || "never verified".to_owned(),
            |at| format!("verified {}", utc(at)),
        );
        let ticked = self.selected.contains(&server.connection);
        let tick = connection.clone();
        h_flex()
            .id(item("agent-server", &server.connection))
            .test_support()
            .gap_3()
            .p_3()
            .items_center()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .child(
                Checkbox::new(item("agent-select", &server.connection))
                    .checked(ticked)
                    .on_click(cx.listener(move |tab, on: &bool, _, cx| {
                        Self::toggle(&mut tab.selected, &tick, *on);
                        cx.notify();
                    })),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .font_semibold()
                            .truncate()
                            .child(format!("{}@{}", server.user, server.host)),
                    )
                    .child(
                        div()
                            .id(item(&format!("agent-server-{slug}"), &server.connection))
                            .test_support()
                            .text_xs()
                            .text_color(colour)
                            .child(state),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!(
                                "{} · added {} by {} · {verified}",
                                server.connection,
                                utc(server.added_at_ms),
                                server.added_by_device
                            )),
                    ),
            )
            .child(
                Button::new(item("agent-deny", &server.connection))
                    .small()
                    .danger()
                    .label("Deny access")
                    .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        view.update(cx, |tab, cx| tab.deny(&connection, cx));
                    }),
            )
            .into_any_element()
    }

    /// The chat accesses that are active, each with the button that ends it.
    fn accesses(
        &self,
        grants: &[AgentAccessGrant],
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        let rows =
            grants.iter().map(|grant| {
                let (view, id) = (cx.entity(), grant.grant_id.clone());
                h_flex()
                    .id(item("agent-grant", &grant.grant_id))
                    .test_support()
                    .gap_3()
                    .p_3()
                    .items_center()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(div().font_semibold().truncate().child(format!(
                                "Agent {} · chat {}",
                                grant.agent_id, grant.chat_id
                            )))
                            .child(div().text_xs().text_color(theme.muted_foreground).child(
                                format!(
                                    "{} · until {}",
                                    grant.targets.join(", "),
                                    utc(grant.expires_at_ms)
                                ),
                            )),
                    )
                    .child(
                        Button::new(item("agent-grant-revoke", &grant.grant_id))
                            .small()
                            .danger()
                            .label("Revoke")
                            .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                view.update(cx, |tab, cx| tab.revoke(&id, cx));
                            }),
                    )
            });
        v_flex()
            .id("agent-grants")
            .test_support()
            .gap_2()
            .child(div().font_semibold().child("Active chat accesses"))
            .when(grants.is_empty(), |column| {
                column.child(note("No agent holds access to your servers now.", cx))
            })
            .children(rows)
            .into_any_element()
    }
}

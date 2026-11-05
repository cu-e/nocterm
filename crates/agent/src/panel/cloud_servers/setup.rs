//! A Cloud chat's explicit server onboarding. The captured account source guards every
//! operation; the existing Hubs service owns CA edits and the original directory opens SSH.
use gpui_kit::{
    App, Context, Entity, EventEmitter, Subscription, Task, TestSupportExt as _, WeakEntity,
    Window,
    component::{ActiveTheme as _, Disableable as _, Sizable as _, button::Button, h_flex, v_flex},
    div,
    prelude::*,
};
use nocterm_agent_chat::conversation::{ConversationError, ConversationSource};
use nocterm_hubs::{Hubs, HubsEvent};
use nocterm_hubs_model::{HubId, ServerTrust};
use nocterm_workspace::{ConnectionDirectory, ConnectionSummary, Workspace};
use std::{collections::BTreeSet, rc::Rc};

use super::item;

/// Original account and destination of the server onboarding controls.
#[derive(Clone)]
pub(in crate::panel) struct SetupAccess {
    hubs: Entity<Hubs>,
    hub: HubId,
    workspace: WeakEntity<Workspace>,
    directory: Option<Rc<dyn ConnectionDirectory>>,
    source: Rc<dyn ConversationSource>,
}
impl SetupAccess {
    pub(in crate::panel) fn capture(
        access: &crate::CloudAgentsAccess,
        workspace: &WeakEntity<Workspace>,
        source: Rc<dyn ConversationSource>,
        cx: &App,
    ) -> Option<Self> {
        source.with_view(cx, &mut |_| {}).ok()?;
        Some(Self {
            hubs: access.hubs().clone(),
            hub: access.shown_hub(cx)?,
            workspace: workspace.clone(),
            directory: workspace.upgrade()?.read(cx).connection_directory(),
            source,
        })
    }
    fn check(&self, cx: &App) -> Result<(), ConversationError> {
        self.source.with_view(cx, &mut |_| {})
    }
}

enum Registry {
    Loading,
    Ready(BTreeSet<String>),
    Failed(String),
}

/// A successful CA edit means the containing binding form must list its choices again.
pub(super) enum SetupChanged {
    Access,
    Notice,
}

pub(super) struct CloudServerSetup {
    access: SetupAccess,
    connections: Vec<ConnectionSummary>,
    registry: Registry,
    work: Option<Task<()>>,
    error: Option<String>,
    _hub_changed: Subscription,
}
impl EventEmitter<SetupChanged> for CloudServerSetup {}
impl CloudServerSetup {
    pub(super) fn new(access: SetupAccess, cx: &mut Context<Self>) -> Self {
        let _hub_changed = cx.subscribe(&access.hubs, |setup, _, _: &HubsEvent, cx| {
            if let Err(error) = setup.access.check(cx) {
                setup.error = Some(error.to_string());
                cx.emit(SetupChanged::Notice);
                cx.notify();
            }
        });
        Self {
            access,
            connections: Vec::new(),
            registry: Registry::Loading,
            work: None,
            error: None,
            _hub_changed,
        }
    }
    /// A display projection outside the scrollable choices so failures stay visible.
    pub(super) fn notice(&self, cx: &App) -> Option<gpui_kit::AnyElement> {
        let message = self.error.as_deref().or(match &self.registry {
            Registry::Failed(error) => Some(error.as_str()),
            _ => None,
        })?;
        Some(
            div()
                .id("cloud-server-setup-error")
                .test_support()
                .text_xs()
                .text_color(cx.theme().danger)
                .child(message.to_owned())
                .into_any_element(),
        )
    }
    pub(super) fn working(&self) -> bool {
        self.work.is_some()
    }
    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        if self.working() {
            return;
        }
        if let Err(error) = self.access.check(cx) {
            self.registry = Registry::Failed(error.to_string());
            cx.emit(SetupChanged::Notice);
            cx.notify();
            return;
        }
        self.connections = self
            .access
            .directory
            .as_ref()
            .map(|directory| directory.connections(cx))
            .unwrap_or_default();
        self.registry = Registry::Loading;
        self.error = None;
        let listing = self.access.hubs.read(cx).agent_overview(&self.access.hub);
        self.work = Some(cx.spawn(async move |this, cx| {
            let result = listing.await;
            let _ = this.update(cx, |setup, cx| {
                setup.work = None;
                setup.registry = match setup.access.check(cx) {
                    Err(error) => Registry::Failed(error.to_string()),
                    Ok(()) => match result {
                        Ok(overview) => Registry::Ready(
                            overview
                                .servers
                                .into_iter()
                                .filter(|server| {
                                    server.state == ServerTrust::Trusted && !server.needs_ca_update
                                })
                                .map(|server| server.connection)
                                .collect(),
                        ),
                        Err(error) => Registry::Failed(error.to_string()),
                    },
                };
                cx.emit(SetupChanged::Notice);
                cx.notify();
            });
        }));
        cx.emit(SetupChanged::Notice);
        cx.notify();
    }
    fn allow(&mut self, connection: &str, cx: &mut Context<Self>) {
        if self.working() {
            return;
        }
        if let Err(error) = self.access.check(cx) {
            self.error = Some(error.to_string());
            cx.emit(SetupChanged::Notice);
            cx.notify();
            return;
        }
        self.error = None;
        let allowing = self
            .access
            .hubs
            .read(cx)
            .allow_agent_host(&self.access.hub, connection);
        self.work = Some(cx.spawn(async move |this, cx| {
            let result = allowing.await;
            let _ = this.update(cx, |setup, cx| {
                setup.work = None;
                match setup.access.check(cx) {
                    Err(error) => setup.error = Some(error.to_string()),
                    Ok(()) => match result {
                        Ok(()) => {
                            setup.reload(cx);
                            cx.emit(SetupChanged::Access);
                        }
                        Err(error) => setup.error = Some(error.to_string()),
                    },
                }
                cx.emit(SetupChanged::Notice);
                cx.notify();
            });
        }));
        cx.emit(SetupChanged::Notice);
        cx.notify();
    }
    fn connect(&mut self, connection: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.working() {
            return;
        }
        let access = self.access.clone();
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let error = match access.check(cx) {
                Err(error) => Some(error.to_string()),
                Ok(()) => {
                    let opened = access.directory.as_ref().is_some_and(|directory| {
                        directory.open(&connection, &access.workspace, window, cx)
                    });
                    (!opened).then(|| "Could not open this saved server. Open it in Connections, then refresh.".to_owned())
                }
            };
            let _ = owner.update(cx, |setup, cx| {
                setup.error = error;
                cx.emit(SetupChanged::Notice);
                cx.notify();
            });
        });
    }
    fn rows(&self, known: &BTreeSet<String>, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let connected: BTreeSet<_> = self
            .access
            .hubs
            .read(cx)
            .agent_candidates()
            .into_iter()
            .map(|host| host.connection)
            .collect();
        self.connections
            .iter()
            .map(|server| {
                self.row(
                    server,
                    connected.contains(server.id.as_ref()),
                    known.contains(server.id.as_ref()),
                    cx,
                )
            })
            .collect()
    }
    fn row(
        &self,
        server: &ConnectionSummary,
        reachable: bool,
        trusted: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let id = server.id.to_string();
        let open_id = id.clone();
        let connect = Button::new(item("cloud-server-connect", &id))
            .small()
            .label(if reachable { "Reconnect" } else { "Connect" })
            .disabled(self.working())
            .on_click(cx.listener(move |setup, _, window, cx| {
                setup.connect(open_id.clone(), window, cx);
            }));
        let allow = Button::new(item("cloud-server-allow", &id))
            .small()
            .label(if trusted { "Access allowed" } else { "Allow access" })
            .tooltip("Add this account's agent CA to the server's authorized_keys. Then select it above and bind this chat.")
            .disabled(self.working() || !reachable || trusted)
            .on_click(cx.listener(move |setup, _, _, cx| setup.allow(&id, cx)));
        v_flex()
            .id(item("cloud-server-setup-row", &server.id))
            .test_support()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .min_w_0()
                    .child(format!("{} · {}", server.name, server.target)),
            )
            .child(h_flex().gap_2().flex_wrap().child(connect).child(allow))
            .into_any_element()
    }
}
impl Render for CloudServerSetup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.registry {
            Registry::Loading => div()
                .text_xs()
                .child("Reading server access…")
                .into_any_element(),
            Registry::Failed(_) => div()
                .text_xs()
                .child("Refresh to read server access again.")
                .into_any_element(),
            Registry::Ready(known) => v_flex()
                .gap_2()
                .children(self.rows(known, cx))
                .into_any_element(),
        };
        let refresh = Button::new("cloud-server-refresh")
            .small()
            .label("Refresh")
            .disabled(self.working())
            .on_click(cx.listener(|setup, _, _, cx| {
                setup.reload(cx);
                cx.emit(SetupChanged::Access);
            }));
        v_flex()
            .id("cloud-server-setup")
            .test_support()
            .min_w_0()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().text_sm().flex_1().child("Add saved servers"))
                    .child(refresh),
            )
            .child(
                div()
                    .id("cloud-server-help")
                    .test_support()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Connect, then allow your account's SSH CA."),
            )
            .when(self.connections.is_empty(), |view| {
                view.child(div().text_xs().child(
                    "No saved servers in this window. Add one in Connections, then refresh.",
                ))
            })
            .child(content)
    }
}

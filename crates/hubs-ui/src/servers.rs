//! The Servers page of the settings: the known servers in a list, and the selected one in
//! tabs: its name, colour, status and certificate with the ways to sign out of it or remove it,
//! the devices of the account, its vault keys, its agents and providers, and the agent key
//! with the servers connected to the agents.
//!
//! The page edits through the service and never decides who may be trusted: a certificate is
//! shown, never accepted, and the only way out of a changed one is to remove the server. The
//! page owns only which server and which tab are selected; each tab owns its own state.

mod detail;
mod list;
mod tabs;

use std::time::Duration;

use gpui_kit::{
    App, Context, ElementId, Entity, FocusHandle, Focusable, Subscription, Task,
    TestSupportExt as _, Window,
    component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex},
    div,
    prelude::*,
};
use nocterm_hubs::HubSummary;
use nocterm_hubs::{HubStatus, Hubs};
use nocterm_hubs_model::HubId;
use nocterm_ui::{IconName, SettingsPage, SettingsPageSpec};

use crate::installation::Installed;
use crate::layout::note;
use detail::{Access, Tab, TabViews, detail};
use list::server_list;
use tabs::{
    account_keys::AccountKeys, agent_access::AgentAccessTab, agents::AgentsTab, devices::Devices,
    names::NameFields,
};

/// The Servers page of the settings, listed with the other pages by the application.
pub fn settings_page(installed: &Installed) -> SettingsPageSpec {
    let hubs = installed.hubs().clone();
    let agents = installed.hub_agents().cloned();
    SettingsPageSpec::new("servers", "Servers", move |_, cx| {
        let (hubs, agents) = (hubs.clone(), agents.clone());
        cx.new(|cx| ServersPage::new(hubs, agents, cx))
    })
    .with_icon(IconName::Server)
}

/// How often the countdown of an offline server is redrawn.
const TICK: Duration = Duration::from_secs(1);

/// The id of the element of kind `kind` of the server at `address`.
pub(crate) fn item(kind: &str, address: &str) -> ElementId {
    ElementId::Name(format!("{kind}-{address}").into())
}

/// The settings page that lists the known servers and shows one of them.
pub(crate) struct ServersPage {
    hubs: Entity<Hubs>,
    /// The server the person chose; the first one while it is none or no longer listed.
    selected: Option<HubId>,
    tab: Tab,
    names: NameFields,
    devices: Entity<Devices>,
    keys: Entity<AccountKeys>,
    agents: Option<Entity<AgentsTab>>,
    access: Option<Entity<AgentAccessTab>>,
    focus: FocusHandle,
    _redraw: Task<()>,
    _hubs: Subscription,
}

impl ServersPage {
    pub(crate) fn new(
        hubs: Entity<Hubs>,
        agents: Option<Entity<nocterm_hub_agents::HubAgents>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let redraw = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = this.update(cx, |page, cx| {
                    if page.counts_down(cx) {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        Self {
            selected: None,
            tab: Tab::Overview,
            names: NameFields::new(hubs.clone()),
            devices: cx.new(|_| Devices::new(hubs.clone())),
            keys: cx.new(|_| AccountKeys::new(hubs.clone())),
            access: agents
                .clone()
                .map(|service| cx.new(|cx| AgentAccessTab::new(service, cx))),
            agents: agents.map(|service| cx.new(|cx| AgentsTab::new(service, cx))),
            focus: cx.focus_handle(),
            _redraw: redraw,
            _hubs: cx.observe(&hubs, |_, _, cx| cx.notify()),
            hubs,
        }
    }

    /// Whether a server shows a countdown that must be redrawn.
    fn counts_down(&self, cx: &App) -> bool {
        self.hubs
            .read(cx)
            .list()
            .iter()
            .any(|summary| matches!(summary.status, HubStatus::Offline { .. }))
    }

    /// Shows `hub`, from its first tab.
    fn select_hub(&mut self, hub: HubId, cx: &mut Context<Self>) {
        if self.selected.as_ref() != Some(&hub) {
            self.tab = Tab::Overview;
        }
        self.selected = Some(hub);
        cx.notify();
    }

    fn select_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }
}

impl SettingsPage for ServersPage {
    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.names.flush(window, cx);
    }
}

impl Focusable for ServersPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The server whose `wanted` tab is open, while this device can reach its account.
fn on_show(shown: Option<&HubSummary>, current: Tab, wanted: Tab) -> Option<HubId> {
    shown
        .filter(|summary| current == wanted && Access::of(&summary.status) == Access::SignedIn)
        .map(|summary| summary.id.clone())
}

impl Render for ServersPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.names.sync(window, cx);
        let summaries = self.hubs.read(cx).list();
        let shown = self
            .selected
            .as_ref()
            .and_then(|hub| summaries.iter().find(|summary| &summary.id == hub))
            .or(summaries.first());
        // The devices are read only while their tab is on show, and again each time it is.
        let devices_on_show = on_show(shown, self.tab, Tab::Devices);
        self.devices
            .update(cx, |devices, cx| match &devices_on_show {
                Some(hub) => devices.show(hub, cx),
                None => devices.hide(),
            });
        if let Some(hub) = on_show(shown, self.tab, Tab::Keys) {
            self.keys.update(cx, |keys, cx| keys.show(&hub, cx));
        }
        // The agents and the agent servers are read again each time their tab is on show.
        let agents_on_show = on_show(shown, self.tab, Tab::Agents);
        if let Some(tab) = &self.agents {
            tab.update(cx, |tab, cx| match &agents_on_show {
                Some(hub) => tab.show(hub, cx),
                None => tab.hide(),
            });
        }
        let access_on_show = on_show(shown, self.tab, Tab::AgentAccess);
        if let Some(tab) = &self.access {
            tab.update(cx, |tab, cx| match &access_on_show {
                Some(hub) => tab.show(hub, cx),
                None => tab.hide(),
            });
        }
        let body = shown.and_then(|summary| {
            let name = self.names.get(&summary.id)?;
            Some(detail(
                summary,
                &name,
                self.tab,
                TabViews {
                    devices: &self.devices,
                    keys: &self.keys,
                    agents: self.agents.as_ref(),
                    access: self.access.as_ref(),
                },
                cx.weak_entity(),
                &self.hubs,
                cx,
            ))
        });
        let list = server_list(&summaries, shown.map(|summary| &summary.id), cx);
        v_flex()
            .id("servers-page")
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .p_4()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_lg().font_semibold().child("Servers"))
                    .child(note(
                        "Nocterm hubs this device is signed in to. Credentials stay in a private file on this device.",
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .gap_4()
                    .items_start()
                    .child(list)
                    .children(body)
                    .when(summaries.is_empty(), |page| {
                        page.child(div().id("servers-empty").test_support().child(note(
                            "No servers yet. Sign in to one to use your hub from this computer.",
                            cx,
                        )))
                    }),
            )
            .text_color(cx.theme().foreground)
    }
}

#[cfg(test)]
#[path = "servers_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "servers_agents_tests.rs"]
mod agent_tests;

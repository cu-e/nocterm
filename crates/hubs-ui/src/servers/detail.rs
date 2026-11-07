//! The selected server on the right of the Servers page: its name and the tabs.

use gpui_kit::{
    App, ClickEvent, Entity, TestSupportExt as _, WeakEntity, Window,
    component::{
        ActiveTheme as _, Icon, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        tab::{Tab as TabButton, TabBar},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_hubs::{HubStatus, HubSummary, Hubs};
use nocterm_ui::SettingsTextField;

use super::tabs::{
    account_keys::AccountKeys, agent_access::AgentAccessTab, agents::AgentsTab, devices::Devices,
    overview::overview, trust::trust,
};
use super::{ServersPage, item};
use crate::color::tint;
use crate::layout::note;
use crate::status::status_icon;

/// The tabs of a server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tab {
    Overview,
    Devices,
    Keys,
    Agents,
    AgentAccess,
}

impl Tab {
    const ALL: [Self; 5] = [
        Self::Overview,
        Self::Devices,
        Self::Keys,
        Self::Agents,
        Self::AgentAccess,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Devices => "Devices",
            Self::Keys => "Keys",
            Self::Agents => "Agents",
            Self::AgentAccess => "Agent access",
        }
    }
}

/// The views of the tabs that keep their own state.
pub(super) struct TabViews<'a> {
    pub(super) devices: &'a Entity<Devices>,
    pub(super) keys: &'a Entity<AccountKeys>,
    pub(super) agents: Option<&'a Entity<AgentsTab>>,
    pub(super) access: Option<&'a Entity<AgentAccessTab>>,
}

/// Whether the account of a server can be reached from this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Access {
    /// This device holds credentials.
    SignedIn,
    /// Signing in comes first.
    SignedOut,
    /// The certificate or the protocol stands in the way; the Overview says which.
    Blocked,
}

impl Access {
    pub(super) fn of(status: &HubStatus) -> Self {
        match status {
            HubStatus::SignedOut | HubStatus::Revoked => Self::SignedOut,
            HubStatus::TrustFailed { .. } | HubStatus::UpgradeRequired { .. } => Self::Blocked,
            HubStatus::Online | HubStatus::Connecting | HubStatus::Offline { .. } => Self::SignedIn,
        }
    }
}

/// The server's name and the tab `tab` of it.
pub(super) fn detail(
    summary: &HubSummary,
    name: &SettingsTextField,
    tab: Tab,
    views: TabViews<'_>,
    page: WeakEntity<ServersPage>,
    hubs: &Entity<Hubs>,
    cx: &App,
) -> gpui_kit::AnyElement {
    let key = summary.address.to_string();
    let theme = cx.theme();
    let mark = summary
        .color
        .map_or(theme.muted_foreground, |color| tint(color, cx));
    let selected = Tab::ALL.iter().position(|each| *each == tab).unwrap_or(0);
    let tabs = TabBar::new("servers-tabs")
        .underline()
        .selected_index(selected)
        .children(Tab::ALL.map(|tab| TabButton::new().label(tab.title())))
        .on_click(move |index, _, cx| {
            let tab = Tab::ALL[*index];
            let _ = page.update(cx, |page, cx| page.select_tab(tab, cx));
        });
    let Some(record) = hubs.read(cx).record(&summary.id).cloned() else {
        return div().into_any_element();
    };
    let body = match (tab, Access::of(&summary.status)) {
        (Tab::Overview, _) => overview(summary, &record, name, hubs, cx),
        (_, Access::SignedOut) => signed_out(summary, hubs, &key, cx),
        (_, Access::Blocked) => blocked(&key, cx),
        (Tab::Devices, Access::SignedIn) => views.devices.clone().into_any_element(),
        (Tab::Keys, Access::SignedIn) => trust(summary, hubs, views.keys, cx),
        (Tab::Agents, Access::SignedIn) => match views.agents {
            Some(tab) => tab.clone().into_any_element(),
            None => without_agents(cx),
        },
        (Tab::AgentAccess, Access::SignedIn) => match views.access {
            Some(tab) => tab.clone().into_any_element(),
            None => without_agents(cx),
        },
    };
    v_flex()
        .id(item("server", &key))
        .test_support()
        .flex_1()
        .min_w_0()
        .gap_3()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(Icon::new(status_icon(&summary.status)).text_color(mark))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_semibold()
                        .child(summary.label.clone()),
                ),
        )
        .child(tabs)
        .child(body)
        .into_any_element()
}

/// The empty state of a tab that needs the account, with the way to sign in.
fn signed_out(
    summary: &HubSummary,
    hubs: &Entity<Hubs>,
    key: &str,
    cx: &App,
) -> gpui_kit::AnyElement {
    let (hubs, id) = (hubs.clone(), summary.id.clone());
    v_flex()
        .id(item("server-tab-signed-out", key))
        .test_support()
        .gap_2()
        .items_start()
        .child(note(
            "This device is not signed in to this server. Sign in to see this.",
            cx,
        ))
        .child(
            Button::new(item("server-tab-sign-in", key))
                .primary()
                .label("Sign in")
                .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    crate::sign_in::start(&hubs, Some(&id), None, window, cx);
                }),
        )
        .into_any_element()
}

/// The agent tabs of an application composed without the agents service.
fn without_agents(cx: &App) -> gpui_kit::AnyElement {
    div()
        .child(note("This build has no hub agents.", cx))
        .into_any_element()
}

/// The empty state of a tab while the server cannot be trusted or spoken to.
fn blocked(key: &str, cx: &App) -> gpui_kit::AnyElement {
    div()
        .id(item("server-tab-blocked", key))
        .test_support()
        .child(note(
            "This is not available while the server cannot be reached safely. The Overview tab says why.",
            cx,
        ))
        .into_any_element()
}

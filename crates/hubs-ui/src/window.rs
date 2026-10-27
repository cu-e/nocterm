//! Composing the workspace of a hub window.

use std::rc::Rc;

use gpui_kit::{App, AppContext as _, Context, Entity, WeakEntity, Window};
use nocterm_hubs::HubWindow;
use nocterm_hubs_model::{Environment, HubId, HubWindowMemory, OrgId};
use nocterm_workspace::{PanelRegistration, RegistrationError, Workspace};

use crate::installation::Installed;
use crate::menu::install_menu;
use crate::ports::{EnvironmentOpener, HubMemoryKeeper, HubTerminalOpener};
use crate::remembered::Remembered;
use crate::sessions::{NewSessionMenu, SessionsPanel};
use crate::tabs::{HubTabs, TabSite};
use crate::{OpenHubSessions, plugin};

/// Why a workspace could not be made a hub window.
#[derive(Debug, thiserror::Error)]
pub enum HubWindowError {
    #[error("no such server: {0}")]
    UnknownHub(HubId),
    #[error("the sessions panel could not be registered")]
    Panel(#[from] RegistrationError),
}

/// What the application gives a hub window to act with.
pub struct HubWindowPorts {
    environments: Rc<dyn EnvironmentOpener>,
    terminals: Rc<dyn HubTerminalOpener>,
    memory: Rc<dyn HubMemoryKeeper>,
}

impl HubWindowPorts {
    /// `environments` opens other environments from the environment menu, `terminals` opens the
    /// tabs of the sessions, and `memory` keeps the organization and sessions of the window for
    /// the next start.
    pub fn new(
        environments: Rc<dyn EnvironmentOpener>,
        terminals: Rc<dyn HubTerminalOpener>,
        memory: Rc<dyn HubMemoryKeeper>,
    ) -> Self {
        Self {
            environments,
            terminals,
            memory,
        }
    }
}

/// Makes `workspace` the window of `hub`: the sessions panel, the menu behind "+" that starts a
/// session on a machine, and the environment menu. Tabs open through `ports`.
///
/// `restored` is what the window kept of the hub before a restart: it shows that organization
/// if the account still belongs to it, and attaches tabs again to the sessions that still run,
/// as soon as the hub lists the sessions of their organization (at once if it can, later if it
/// is not reachable yet). A session the hub no longer lists is forgotten without a word.
///
/// The action [`OpenHubSessions`] is registered even when the workspace refuses the panel.
pub fn register_hub_window(
    installed: &Installed,
    hub: HubId,
    ports: HubWindowPorts,
    restored: HubWindowMemory,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), HubWindowError> {
    let hubs = installed.hubs().clone();
    let Some(state) = hubs.update(cx, |hubs, cx| hubs.window(&hub, cx)) else {
        return Err(HubWindowError::UnknownHub(hub));
    };
    install_menu(
        installed,
        Environment::Hub(hub.clone()),
        ports.environments,
        workspace,
        window,
        cx,
    );
    if let Some(organization) = restored.organization() {
        state.update(cx, |state, cx| state.prefer_org(organization.clone(), cx));
    }
    let remembered = cx.new(|_| Remembered::new(hub.clone(), ports.memory, restored));
    let site = TabSite::new(hubs.clone(), hub, cx.weak_entity(), ports.terminals);
    let tabs = HubTabs::new(site.clone(), remembered.clone());
    follow_hub(&state, site, &remembered, window, cx);
    let owner = cx.weak_entity();
    let menu = cx.new(|cx| NewSessionMenu::new(state.clone(), tabs.clone(), owner, cx));
    workspace.set_new_tab_menu(menu, cx);
    workspace.register_action(|workspace, _: &OpenHubSessions, window, cx| {
        workspace.toggle_panel(&plugin::SESSIONS.id, window, cx);
    });
    workspace.register_panel(
        PanelRegistration::new(&plugin::SESSIONS, move |workspace, _, cx| {
            let _ = workspace;
            let handle = cx.entity();
            cx.new(|cx| SessionsPanel::new(hubs.clone(), state.clone(), tabs.clone(), &handle, cx))
        }),
        window,
        cx,
    )?;
    Ok(())
}

/// Lets the memory follow what the hub window shows, and attach remembered sessions again when
/// the hub lists them. The watch belongs to the memory, so it ends with the window.
///
/// The subscription holds the memory weakly: the tabs, which hold it, are made from the pieces
/// of `site` when an event arrives.
fn follow_hub(
    state: &Entity<HubWindow>,
    site: TabSite,
    remembered: &Entity<Remembered>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let weak = remembered.downgrade();
    let on_event = {
        let (site, weak) = (site.clone(), weak.clone());
        move |state: Entity<HubWindow>,
              _: &nocterm_hubs::HubWindowEvent,
              window: &mut Window,
              cx: &mut App| {
            settle(&state, &site, &weak, window, cx);
        }
    };
    let watch = window.subscribe(state, cx, on_event);
    remembered.update(cx, |remembered, _| remembered.watching(watch));
    // A hub that has listed its sessions already raises no event: look once the workspace is
    // composed, which is also when a tab can open.
    let state = state.clone();
    window.defer(cx, move |window, cx| {
        settle(&state, &site, &weak, window, cx);
    });
}

/// Brings the memory up to date with the hub window and, once the account is known, looks for
/// the remembered sessions of each organization they belong to.
fn settle(
    state: &Entity<HubWindow>,
    site: &TabSite,
    remembered: &WeakEntity<Remembered>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(remembered) = remembered.upgrade() else {
        return;
    };
    let (organization, member_of) = state.read_with(cx, |state, _| {
        (
            state.org().cloned(),
            state
                .account()
                .map(|account| account.orgs.iter().map(|org| org.id.clone()).collect()),
        )
    });
    if let Some(organization) = organization {
        remembered.update(cx, |remembered, cx| {
            remembered.organization_shown(organization, cx);
        });
    }
    let Some(member_of): Option<Vec<OrgId>> = member_of else {
        return;
    };
    for org in remembered.read(cx).waiting_orgs() {
        if member_of.contains(&org) {
            reattach(state, site, &remembered, org, window, cx);
        } else {
            // The account left the organization: nothing can be attached there any more.
            remembered.update(cx, |remembered, cx| {
                remembered.reattachable(&org, &[], cx);
            });
        }
    }
}

/// Reads the sessions of `org` and attaches the remembered ones of it that still run. A read
/// that fails leaves them remembered; the next change of the hub window tries again.
fn reattach(
    state: &Entity<HubWindow>,
    site: &TabSite,
    remembered: &Entity<Remembered>,
    org: OrgId,
    window: &mut Window,
    cx: &mut App,
) {
    if !remembered.update(cx, |remembered, _| remembered.begin_fetch(&org)) {
        return;
    }
    let listing = state.update(cx, |state, cx| state.list_sessions(org.clone(), cx));
    let (site, remembered) = (site.clone(), remembered.downgrade());
    window
        .spawn(cx, async move |cx| {
            let listed = listing.await;
            let _ = cx.update(|window, cx| {
                let Some(remembered) = remembered.upgrade() else {
                    return;
                };
                remembered.update(cx, |remembered, _| remembered.end_fetch(&org));
                let Ok(listed) = listed else {
                    return;
                };
                let found = remembered.update(cx, |remembered, cx| {
                    remembered.reattachable(&org, &listed, cx)
                });
                let tabs = HubTabs::new(site, remembered.clone());
                for session in &found {
                    if !tabs.open_existing(session, window, cx) {
                        remembered.update(cx, |remembered, cx| remembered.tab_failed(session, cx));
                    }
                }
            });
        })
        .detach();
}

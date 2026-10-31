//! Composes the Cloud agents service with the known hubs: it is installed once, and follows
//! which hubs are signed in so only those are listed.
use std::sync::Arc;

use gpui_kit::App;
use nocterm_cloud_agents_model::CloudChatLauncher;
use nocterm_hubs::{HubStatus, HubsEvent};

use crate::bootstrap::PendingHubs;

/// Installs the service over `deps` and keeps its tracked hubs equal to the hubs this device is
/// signed in to. Returns the access the agent feature offers, or none without hubs or ports.
pub(crate) fn install(
    deps: Option<nocterm_cloud_agents::Deps>,
    chats: Option<Arc<dyn CloudChatLauncher>>,
    hubs: Option<&PendingHubs>,
    cx: &mut App,
) -> Option<nocterm_agent::CloudAgentsAccess> {
    let (deps, hubs) = (deps?, hubs?.hubs.hubs().clone());
    let service = nocterm_cloud_agents::install(deps, cx);
    let agents = service.agents().clone();
    let follow = {
        let (agents, hubs) = (agents.clone(), hubs.clone());
        move |cx: &mut App| {
            let summaries = hubs.read(cx).list();
            agents.update(cx, |agents, cx| {
                for hub in summaries {
                    match hub.status {
                        HubStatus::Online => agents.track(&hub.id, cx),
                        HubStatus::SignedOut | HubStatus::Revoked => agents.untrack(&hub.id, cx),
                        _ => {}
                    }
                }
            });
        }
    };
    let on_change = follow.clone();
    cx.subscribe(&hubs, move |_, HubsEvent::Changed, cx| on_change(cx))
        .detach();
    follow(cx);
    let access = nocterm_agent::CloudAgentsAccess::new(agents, hubs);
    Some(match chats {
        Some(chats) => access.with_chats(chats),
        None => access,
    })
}

//! The "Hubs" commands of the command palette: open a server in a new window and sign out of
//! one. Each asks which server in a picker, then does what the menu and the Servers page do.

use gpui_kit::{App, Entity, Window};
use nocterm_hubs::{HubStatus, HubSummary, Hubs};
use nocterm_hubs_model::{Environment, HubId};
use nocterm_ui::{PickerOption, open_choice_picker};

use crate::ports::Placement;
use crate::status::status_chip;
use crate::switch::Switch;
use crate::{manage, problems, sign_in};

/// Opens `summary` the way its status allows: as an environment when the server can be
/// reached, in the sign-in dialog when it needs a sign-in, or as an explanation when it is in
/// trouble that the user has to deal with.
pub(crate) fn open_server(
    hubs: Entity<Hubs>,
    summary: HubSummary,
    placement: Placement,
    switch: Switch,
    window: &mut Window,
    cx: &mut App,
) {
    match summary.status.clone() {
        HubStatus::Online | HubStatus::Connecting | HubStatus::Offline { .. } => {
            switch.open(Environment::Hub(summary.id), placement, window, cx);
        }
        HubStatus::SignedOut | HubStatus::Revoked => {
            sign_in::start(&hubs, Some(&summary.id), Some(switch), window, cx);
        }
        HubStatus::TrustFailed { pinned, presented } => {
            problems::open_trust_failed(hubs, summary, pinned, presented, window, cx);
        }
        HubStatus::UpgradeRequired { hub, client } => {
            problems::open_upgrade_required(&summary, hub, client, window, cx);
        }
    }
}

/// Whether the server holds credentials of this device that signing out would erase. A server
/// whose certificate changed has none to offer here: removing it is the way out, and signing
/// out would hide the warning until the next connection attempt.
pub(crate) fn can_sign_out(summary: &HubSummary) -> bool {
    !matches!(
        summary.status,
        HubStatus::SignedOut | HubStatus::Revoked | HubStatus::TrustFailed { .. }
    )
}

fn options(servers: Vec<HubSummary>, cx: &App) -> Vec<PickerOption<HubId>> {
    servers
        .into_iter()
        .map(|summary| {
            let (status, _) = status_chip(&summary.status, cx);
            let label = format!("{} · {} · {status}", summary.label, summary.address);
            PickerOption::new(label, false, summary.id)
        })
        .collect()
}

/// Asks which known server to open in a new window.
pub(crate) fn pick_server_to_open(
    hubs: &Entity<Hubs>,
    switch: Switch,
    window: &mut Window,
    cx: &mut App,
) {
    let servers = hubs.read(cx).list();
    if servers.is_empty() {
        sign_in::start(hubs, None, Some(switch), window, cx);
        return;
    }
    let picks = options(servers, cx);
    let hubs = hubs.clone();
    open_choice_picker(
        "Open a server in a new window",
        picks,
        move |id, window, cx| {
            let Some(summary) = hubs.read(cx).summary(&id) else {
                return;
            };
            open_server(
                hubs.clone(),
                summary,
                Placement::NewWindow,
                switch.clone(),
                window,
                cx,
            );
        },
        window,
        cx,
    );
}

/// Asks which signed-in server to sign out of.
pub(crate) fn pick_server_to_sign_out(hubs: &Entity<Hubs>, window: &mut Window, cx: &mut App) {
    let servers: Vec<HubSummary> = hubs
        .read(cx)
        .list()
        .into_iter()
        .filter(can_sign_out)
        .collect();
    if servers.is_empty() {
        nocterm_ui::notice::info(
            window,
            cx,
            "hub-manage",
            "No server to sign out of",
            "This device is not signed in to any server.".to_owned(),
        );
        return;
    }
    let picks = options(servers, cx);
    let hubs = hubs.clone();
    open_choice_picker(
        "Sign out of a server",
        picks,
        move |id, window, cx| manage::sign_out(&hubs, &id, window, cx),
        window,
        cx,
    );
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;

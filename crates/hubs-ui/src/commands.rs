//! The "Hubs" command of the command palette that signs out of a server. It asks which server
//! in a picker, then does what the Servers page does.

use gpui_kit::{App, Entity, Window};
use nocterm_hubs::{HubStatus, HubSummary, Hubs};
use nocterm_hubs_model::HubId;
use nocterm_ui::{PickerOption, open_choice_picker};

use crate::status::status_chip;
use crate::manage;

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

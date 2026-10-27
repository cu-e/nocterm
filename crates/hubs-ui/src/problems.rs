//! The dialogs for hubs that cannot be signed in to: a certificate that is not the trusted
//! one, and a protocol this Nocterm does not speak. Neither offers a way to accept what the
//! hub presents.

use gpui_kit::{
    App, ClickEvent, Entity, Window,
    component::{
        WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::DialogFooter,
        v_flex,
    },
    prelude::*,
    px,
};
use nocterm_hubs::{HubSummary, Hubs};
use nocterm_hubs_model::Fingerprint;

use crate::layout::{fingerprint_block, note, problem};

/// Tells that the certificate of `hub` is not the trusted one, shows both fingerprints, and
/// offers to forget the server. Signing in again to the same server then asks anew.
pub(crate) fn open_trust_failed(
    hubs: Entity<Hubs>,
    hub: HubSummary,
    pinned: Option<Fingerprint>,
    presented: Fingerprint,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_alert_dialog(cx, move |alert, _, cx| {
        let (summary, hubs) = (hub.clone(), hubs.clone());
        let remove = Button::new("trust-failed-remove")
            .label("Remove this server")
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let id = summary.id.clone();
                hubs.update(cx, |hubs, cx| hubs.remove(&id, cx)).detach();
                window.close_dialog(cx);
            });
        let close = Button::new("trust-failed-close")
            .primary()
            .label("Close")
            .on_click(|_: &ClickEvent, window: &mut Window, cx: &mut App| {
                window.close_dialog(cx);
            });
        let (heading, reason) = match &pinned {
            Some(_) => (
                "The certificate changed",
                "The server presents another certificate than the one pinned when it was added. Nocterm will not connect until you have checked why.",
            ),
            None => (
                "The certificate is not trusted",
                "The server's certificate is not issued by a public authority and none is pinned. Nocterm will not connect.",
            ),
        };
        let mut details = v_flex().gap_3().child(problem(reason.to_owned(), cx));
        if let Some(pinned) = &pinned {
            details = details.child(fingerprint_block("Pinned", pinned, cx));
        }
        details = details
            .child(fingerprint_block("Presented now", &presented, cx))
            .child(note(
                "Remove the server and sign in again only if you can confirm the new fingerprint with whoever runs it.",
                cx,
            ));
        alert
            .title(format!("{}: {heading}", hub.label))
            .width(px(480.))
            .description(details)
            .on_ok(|_, _, _| true)
            .footer(DialogFooter::new().child(remove).child(close))
    });
}

/// Tells that `hub` and this client speak different protocol versions.
pub(crate) fn open_upgrade_required(
    hub: &HubSummary,
    hub_version: u32,
    client_version: u32,
    window: &mut Window,
    cx: &mut App,
) {
    let label = hub.label.clone();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let newer = if hub_version > client_version {
            "Update Nocterm to connect."
        } else {
            "The server needs an update before this Nocterm can connect."
        };
        alert
            .title(format!("{label}: update needed"))
            .description(format!(
                "The server speaks protocol {hub_version} and this Nocterm speaks {client_version}. {newer}"
            ))
            .ok_text("Close")
            .show_cancel(false)
    });
}

//! What the user does to a server they already know: sign out, remove, rename, colour. Each
//! starts the service call and tells the user when it did not work; the service keeps the
//! rules.

use gpui_kit::{
    App, ClickEvent, Entity, Task, Window,
    component::{
        WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::DialogFooter,
    },
    prelude::*,
};
use nocterm_hubs::{HubSummary, Hubs, HubsError};
use nocterm_hubs_model::{HubColor, HubId};

use crate::messages::hubs_problem;

/// Shows a notice titled `title` when `task` fails, once it has finished.
pub(crate) fn report(
    task: Task<Result<(), HubsError>>,
    title: &'static str,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            if let Err(error) = task.await {
                cx.update(|window, cx| {
                    nocterm_ui::notice::error(
                        window,
                        cx,
                        "hub-manage",
                        title,
                        hubs_problem(&error),
                    );
                })
                .ok();
            }
        })
        .detach();
}

/// Ends this device's session on the server. The server stays in the list, signed out.
pub(crate) fn sign_out(hubs: &Entity<Hubs>, hub: &HubId, window: &mut Window, cx: &mut App) {
    let signing_out = hubs.update(cx, |hubs, cx| hubs.sign_out(hub, cx));
    report(signing_out, "Could not sign out", window, cx);
}

/// Forgets the server and its credentials.
pub(crate) fn remove(hubs: &Entity<Hubs>, hub: &HubId, window: &mut Window, cx: &mut App) {
    let removing = hubs.update(cx, |hubs, cx| hubs.remove(hub, cx));
    report(removing, "Could not remove the server", window, cx);
}

/// Marks the server with `color`, or clears the mark.
pub(crate) fn recolor(
    hubs: &Entity<Hubs>,
    hub: &HubId,
    color: Option<HubColor>,
    window: &mut Window,
    cx: &mut App,
) {
    let marking = hubs.update(cx, |hubs, cx| hubs.set_color(hub, color, cx));
    report(marking, "Could not change the colour", window, cx);
}

/// Asks before removing a server: it leaves this device's list and its credentials are
/// erased. Sessions on the server itself are not touched.
pub(crate) fn confirm_remove(
    hubs: Entity<Hubs>,
    hub: HubSummary,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (hubs, id) = (hubs.clone(), hub.id.clone());
        let confirm = Button::new("remove-server-confirm")
            .danger()
            .label("Remove server")
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                window.close_dialog(cx);
                remove(&hubs, &id, window, cx);
            });
        let keep = Button::new("remove-server-keep")
            .primary()
            .label("Keep it")
            .on_click(|_: &ClickEvent, window: &mut Window, cx: &mut App| {
                window.close_dialog(cx);
            });
        alert
            .title(format!("Remove {}?", hub.label))
            .description(format!(
                "{} leaves the list and this device is signed out of it. Its sessions keep running on the server.",
                hub.address
            ))
            .on_ok(|_, _, _| true)
            .footer(DialogFooter::new().child(confirm).child(keep))
    });
}

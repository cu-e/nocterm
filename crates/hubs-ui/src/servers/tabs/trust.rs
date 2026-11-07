//! The Keys tab of a server: the account keys, opened and closed on this device, and a
//! recovery key that was never confirmed as saved.

use gpui_kit::{
    App, ClickEvent, Entity, TestSupportExt as _, Window,
    component::{button::Button, h_flex, v_flex},
    prelude::*,
};
use nocterm_hubs::{HubSummary, Hubs};

use super::account_keys::AccountKeys;
use crate::layout::{field, note};
use crate::servers::item;

/// The account keys of the server and, while one was never saved, the offer of a new recovery
/// key.
pub(in crate::servers) fn trust(
    summary: &HubSummary,
    hubs: &Entity<Hubs>,
    keys: &Entity<AccountKeys>,
    cx: &App,
) -> gpui_kit::AnyElement {
    let key = summary.address.to_string();
    let recovery = recovery_key(summary, hubs, &key, cx);
    v_flex()
        .id(item("server-trust-tab", &key))
        .test_support()
        .gap_3()
        .child(field("Vault keys", keys.clone()))
        .children(recovery)
        .into_any_element()
}

/// The offer of a new recovery key, while the one made with the account keys on this device was
/// never confirmed as saved and the keys are open.
fn recovery_key(
    summary: &HubSummary,
    hubs: &Entity<Hubs>,
    key: &str,
    cx: &App,
) -> Option<gpui_kit::AnyElement> {
    if !hubs.read(cx).recovery_key_unconfirmed(&summary.id) {
        return None;
    }
    let offer = hubs.read(cx).recovery_key_offer(&summary.id).ok()?;
    let (hubs, id) = (hubs.clone(), summary.id.clone());
    Some(
        field(
            "Recovery key",
            v_flex()
                .id(item("server-recovery", key))
                .test_support()
                .gap_2()
                .child(note(
                    "The recovery key made with your account keys was never confirmed as saved. Issue a new one so you can still open them if you lose your password.",
                    cx,
                ))
                .child(
                    h_flex().justify_end().child(
                        Button::new(item("server-recovery-issue", key))
                            .label("Issue a new recovery key")
                            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                crate::sign_in::start_reissue(&hubs, &id, offer.clone(), window, cx);
                            }),
                    ),
                ),
        )
        .into_any_element(),
    )
}

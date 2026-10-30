//! The Keys & trust tab of a server: the account keys and the administrators whose keys wait
//! for the person's confirmation.

use gpui_kit::{
    App, ClickEvent, Entity, TestSupportExt as _, Window,
    component::{button::Button, h_flex, v_flex},
    prelude::*,
};
use nocterm_hubs::{HubSummary, Hubs};

use super::admin_keys::admin_keys;
use crate::layout::{field, note};
use crate::servers::item;

/// What a person confirms about the account of the server: a recovery key that was never
/// saved, and the keys of organization administrators.
pub(in crate::servers) fn trust(
    summary: &HubSummary,
    hubs: &Entity<Hubs>,
    cx: &App,
) -> gpui_kit::AnyElement {
    let key = summary.address.to_string();
    let recovery = recovery_key(summary, hubs, &key, cx);
    let administrators = admin_keys(summary, hubs, &key, cx);
    let nothing = recovery.is_none() && administrators.is_none();
    v_flex()
        .id(item("server-trust-tab", &key))
        .test_support()
        .gap_3()
        .children(recovery)
        .children(administrators)
        .when(nothing, |tab| {
            tab.child(
                gpui_kit::div()
                    .id(item("server-keys-settled", &key))
                    .test_support()
                    .child(note(
                        "Nothing waits for you here. Account keys that are closed on this device show their offers once the vault is unlocked.",
                        cx,
                    )),
            )
        })
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

//! The administrators whose key waits for the person's confirmation, on the Keys & trust tab of a server.
//!
//! The page only shows what the account offers and passes the decision on: it never judges
//! whether a fingerprint is right. A person compares it with the one the administrator sees in
//! `hubctl`, out of band, and confirms only when they are identical.

use gpui_kit::{
    App, ClickEvent, Entity, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_hubs::{HubSummary, Hubs};
use nocterm_hubs_model::AdminKeyOffer;

use crate::layout::{field, note, problem};
use crate::manage;
use crate::servers::item;

/// Fingerprint groups on one line.
const GROUPS_PER_LINE: usize = 8;

/// A fingerprint as lines of groups, which are easier to compare by eye. A text without groups
/// is shown as it is.
pub(in crate::servers) fn grouped(fingerprint: &str) -> String {
    let groups: Vec<&str> = fingerprint.split(['-', ' ']).collect();
    groups
        .chunks(GROUPS_PER_LINE)
        .map(|line| line.join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The section of keys to confirm; nothing while none waits.
pub(super) fn admin_keys(
    summary: &HubSummary,
    hubs: &Entity<Hubs>,
    key: &str,
    cx: &App,
) -> Option<gpui_kit::AnyElement> {
    let pending = hubs.read(cx).pending_admin_keys(&summary.id);
    if pending.is_empty() {
        return None;
    }
    let cards: Vec<_> = pending
        .iter()
        .enumerate()
        .map(|(index, pending)| card(hubs, key, index, pending, cx))
        .collect();
    Some(
        field(
            "Administrator keys",
            v_flex()
                .id(item("server-admin-keys", key))
                .test_support()
                .gap_2()
                .child(note(
                    "These organizations added you to groups, but their groups stay hidden until you confirm the administrator's key. Compare each fingerprint with the one the administrator sees in hubctl, in person or over a call you trust, and confirm only when they are identical.",
                    cx,
                ))
                .children(cards),
        )
        .into_any_element(),
    )
}

/// One administrator: who, the fingerprint, and the button that confirms it.
fn card(
    hubs: &Entity<Hubs>,
    key: &str,
    index: usize,
    offer: &AdminKeyOffer,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let pending = offer.key();
    let changed = pending.replaces.is_some();
    let hubs = hubs.clone();
    let captured = offer.clone();
    let fingerprint = pending.fingerprint.clone();
    let confirm =
        Button::new(item(&format!("server-admin-confirm-{index}"), key)).label(if changed {
            "Confirm the changed key"
        } else {
            "Confirm key"
        });
    let confirm = if changed { confirm.danger() } else { confirm };
    let confirm = confirm.on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
        manage::confirm_admin_key(&hubs, &captured, &fingerprint, window, cx);
    });
    v_flex()
        .id(item(&format!("server-admin-key-{index}"), key))
        .test_support()
        .gap_2()
        .p_2()
        .rounded(theme.radius)
        .border_1()
        .border_color(if changed { theme.danger } else { theme.border })
        .child(div().font_semibold().child(format!(
            "{}: administrator {}",
            pending.organization, pending.admin
        )))
        .children(
            pending
                .replaces
                .as_ref()
                .map(|old| changed_key(key, index, old, cx)),
        )
        .child(fingerprint_text(
            if changed {
                "Offered now"
            } else {
                "Fingerprint"
            },
            &pending.fingerprint,
            item(&format!("server-admin-fingerprint-{index}"), key),
            cx,
        ))
        .child(h_flex().justify_end().child(confirm))
}

/// The warning for a key that is not the one confirmed before.
fn changed_key(key: &str, index: usize, old: &str, cx: &App) -> impl IntoElement {
    v_flex()
        .id(item(&format!("server-admin-changed-{index}"), key))
        .test_support()
        .gap_2()
        .child(problem(
            "The key of this administrator changed. The server now offers another key than the one you confirmed before. That is either a new key made by the administrator or someone tampering with the server: confirm it only if the administrator tells you, out of band, that they changed it."
                .to_owned(),
            cx,
        ))
        .child(fingerprint_text(
            "Confirmed before",
            old,
            item(&format!("server-admin-before-{index}"), key),
            cx,
        ))
}

fn fingerprint_text(
    label: &'static str,
    fingerprint: &str,
    id: gpui_kit::ElementId,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .id(id)
                .test_support()
                .font_family(cx.theme().mono_font_family.clone())
                .text_sm()
                .child(grouped(fingerprint)),
        )
}

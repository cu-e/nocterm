//! One server on the Servers page.

use gpui_kit::{
    App, ClickEvent, ElementId, Entity, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Icon, Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_hubs::{HubStatus, HubSummary, Hubs};
use nocterm_hubs_model::{HubColor, HubRecord, TrustPolicy};
use nocterm_ui::{IconName, SettingsTextField};

use crate::color::tint;
use crate::commands::can_sign_out;
use crate::layout::{field, fingerprint_block, note, problem};
use crate::manage;
use crate::status::{status_chip, status_icon};

/// The id of the element of kind `kind` in the row of the server at `address`.
pub(super) fn item(kind: &str, address: &str) -> ElementId {
    ElementId::Name(format!("{kind}-{address}").into())
}

/// The row of one server: name, colour, status, certificate and what can be done to it.
pub(super) fn server_row(
    summary: &HubSummary,
    record: &HubRecord,
    name: &SettingsTextField,
    hubs: &Entity<Hubs>,
    cx: &App,
) -> gpui_kit::AnyElement {
    let key = summary.address.to_string();
    let theme = cx.theme();
    let (status, status_color) = status_chip(&summary.status, cx);
    let icon_color = summary
        .color
        .map_or(theme.muted_foreground, |color| tint(color, cx));
    v_flex()
        .id(item("server", &key))
        .test_support()
        .gap_3()
        .p_3()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(Icon::new(status_icon(&summary.status)).text_color(icon_color))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_semibold()
                        .child(summary.label.clone()),
                )
                .child(
                    div()
                        .id(item("server-status", &key))
                        .test_support()
                        .text_xs()
                        .text_color(status_color)
                        .child(status),
                ),
        )
        .child(note(summary.address.to_string(), cx))
        .child(field(
            "Name",
            div()
                .id(item("server-name", &key))
                .test_support()
                .child(name.input()),
        ))
        .children(name.error(cx).map(|error| problem(error.to_string(), cx)))
        .child(colors(summary, hubs, &key, cx))
        .child(certificate(summary, record, &key, cx))
        .child(actions(summary, hubs, &key))
        .into_any_element()
}

/// The chooser of the colour that marks the server.
fn colors(summary: &HubSummary, hubs: &Entity<Hubs>, key: &str, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (border, foreground) = (theme.border, theme.foreground);
    let none = {
        let (hubs, id) = (hubs.clone(), summary.id.clone());
        Button::new(item("server-color-none", key))
            .ghost()
            .xsmall()
            .label("None")
            .selected(summary.color.is_none())
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                manage::recolor(&hubs, &id, None, window, cx);
            })
    };
    let swatches = HubColor::ALL.into_iter().map(|color| {
        let (hubs, id) = (hubs.clone(), summary.id.clone());
        let chosen = summary.color == Some(color);
        div()
            .id(item(&format!("server-color-{}", color.name()), key))
            .test_support()
            .size_5()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(tint(color, cx))
            .border_2()
            .border_color(if chosen { foreground } else { border })
            .when(chosen, |swatch| {
                swatch.child(Icon::new(IconName::Check).xsmall().text_color(foreground))
            })
            .on_click(move |_, window, cx| {
                manage::recolor(&hubs, &id, Some(color), window, cx);
            })
    });
    field(
        "Colour",
        h_flex()
            .gap_2()
            .items_center()
            .child(none)
            .children(swatches),
    )
}

/// How the server's certificate is trusted. It is only shown: a certificate that changed is
/// never accepted here, and removing the server is the way to sign in to the new one.
fn certificate(summary: &HubSummary, record: &HubRecord, key: &str, cx: &App) -> impl IntoElement {
    let mut section = v_flex()
        .id(item("server-trust", key))
        .test_support()
        .gap_2();
    match record.trust {
        TrustPolicy::Pinned(fingerprint) => {
            section = section.child(div().id(item("server-pinned", key)).test_support().child(
                fingerprint_block("Pinned certificate (SHA-256)", &fingerprint, cx),
            ));
        }
        TrustPolicy::PublicCa => {
            section = section.child(
                div()
                    .id(item("server-public-ca", key))
                    .test_support()
                    .child(note(
                        "Trusted through a public certificate authority. No certificate is pinned.",
                        cx,
                    )),
            );
        }
    }
    if let HubStatus::TrustFailed { presented, .. } = &summary.status {
        section = section
            .child(problem(
                "The server presents another certificate than the one trusted. Nocterm will not connect. Remove the server and sign in again only if you can confirm the new fingerprint with whoever runs it."
                    .to_owned(),
                cx,
            ))
            .child(
                div()
                    .id(item("server-presented", key))
                    .test_support()
                    .child(fingerprint_block("Presented now", presented, cx)),
            );
    }
    section
}

/// Sign out and remove.
fn actions(summary: &HubSummary, hubs: &Entity<Hubs>, key: &str) -> impl IntoElement {
    let signed_in = can_sign_out(summary);
    let sign_out = signed_in.then(|| {
        let (hubs, id) = (hubs.clone(), summary.id.clone());
        Button::new(item("server-sign-out", key))
            .label("Sign out")
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                manage::sign_out(&hubs, &id, window, cx);
            })
    });
    let remove = {
        let (hubs, summary) = (hubs.clone(), summary.clone());
        Button::new(item("server-remove", key))
            .danger()
            .label("Remove…")
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                manage::confirm_remove(hubs.clone(), summary.clone(), window, cx);
            })
    };
    h_flex()
        .gap_2()
        .justify_end()
        .mt(px(2.))
        .children(sign_out)
        .child(remove)
}

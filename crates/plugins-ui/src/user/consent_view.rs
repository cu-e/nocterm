//! How the consent dialog looks.

use gpui_kit::{
    AnyElement, Context, SharedString, TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        switch::Switch,
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_ui::form;

use super::{
    consent::{Consent, quote},
    port::{Candidate, CapabilityLine, FieldLine, Replaced},
    section::UserPluginsSection,
    section_view::fact,
};

pub(super) fn consent_dialog(
    section: &UserPluginsSection,
    consent: &Consent,
    cx: &mut Context<UserPluginsSection>,
) -> AnyElement {
    let candidate = consent.candidate();
    let title = match &candidate.replaces {
        Some(_) => format!("Replace {} (unverified publisher)", candidate.name),
        None => format!("Install {}?", candidate.name),
    };
    let capabilities: Vec<AnyElement> = candidate
        .capabilities
        .iter()
        .map(|line| capability(consent, line, cx))
        .collect();
    let fields: Vec<AnyElement> = candidate
        .fields
        .iter()
        .map(|line| field(line, cx))
        .collect();
    v_flex()
        .id("user-consent")
        .aria_label(title.clone())
        .test_support()
        .w_full()
        .gap_2()
        .p_4()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(cx.theme().radius)
        .child(div().text_lg().font_semibold().child(title))
        .child(fact(
            "Publisher",
            format!("{} (unverified)", candidate.publisher),
            cx,
        ))
        .child(code_line(candidate, cx))
        .child(versions(candidate, cx))
        .child(form::section("It asks to", capabilities, cx))
        .when(!fields.is_empty(), |dialog| {
            dialog.child(form::section("Settings it adds", fields, cx))
        })
        .when_some(candidate.replaces.as_ref(), |dialog, old| {
            dialog.child(replacement(consent, old, cx))
        })
        .child(answer(section, consent, cx))
        .child(state(consent.is_armed(), cx))
        .into_any_element()
}

/// Whether the package runs code, as a fact on every consent. An update that changes it says so.
fn code_line(candidate: &Candidate, cx: &Context<UserPluginsSection>) -> AnyElement {
    let text = match (
        candidate.runs_code,
        candidate.replaces.as_ref().map(|old| old.runs_code),
    ) {
        (true, Some(false)) => "Yes, new: the installed version runs no code",
        (true, _) => "Yes: a WebAssembly module in a sandbox",
        (false, Some(true)) => "No: the installed version runs code",
        (false, _) => "No",
    };
    h_flex()
        .id("user-consent-code")
        .aria_label(text)
        .test_support()
        .gap_2()
        .text_sm()
        .child(
            div()
                .w_24()
                .text_color(cx.theme().muted_foreground)
                .child("Runs code"),
        )
        .child(div().child(text))
        .into_any_element()
}

/// The version and fingerprint the user compares; both of them for an update.
fn versions(candidate: &Candidate, cx: &Context<UserPluginsSection>) -> AnyElement {
    let new = format!("{} \u{b7} {}", candidate.version, candidate.fingerprint);
    match &candidate.replaces {
        Some(old) => v_flex()
            .child(fact(
                "Installed",
                format!("{} \u{b7} {}", old.version, old.fingerprint),
                cx,
            ))
            .child(fact("New", new, cx))
            .into_any_element(),
        None => fact("Fingerprint", new, cx),
    }
}

/// What an update asks beyond a first install: the data, and a downgrade's confirmation.
fn replacement(
    consent: &Consent,
    old: &Replaced,
    cx: &mut Context<UserPluginsSection>,
) -> AnyElement {
    let keep = Switch::new("user-consent-keep")
        .checked(consent.keeps_data())
        .on_click(cx.listener(|section, keep: &bool, _, cx| {
            if let Some(consent) = section.consent_mut() {
                consent.keep_data(*keep);
            }
            cx.notify();
        }));
    let downgrade = Switch::new("user-consent-downgrade")
        .checked(consent.confirms_downgrade())
        .on_click(cx.listener(|section, sure: &bool, _, cx| {
            if let Some(consent) = section.consent_mut() {
                consent.confirm_downgrade(*sure);
            }
            cx.notify();
        }));
    v_flex()
        .child(form::row(
            "Keep this plugin's data",
            "Off: its values and storage are removed with the old version.",
            keep,
            cx,
        ))
        .when(old.downgrade, |rows| {
            rows.child(form::row(
                "This is an older version",
                "Confirm that you want to go back.",
                downgrade,
                cx,
            ))
        })
        .into_any_element()
}

/// Deny holds focus and is the default; accepting waits for the buttons to wake.
fn answer(
    section: &UserPluginsSection,
    consent: &Consent,
    cx: &mut Context<UserPluginsSection>,
) -> AnyElement {
    let replaces = consent.candidate().replaces.is_some();
    h_flex()
        .gap_2()
        .pt_2()
        .child(
            Button::new("user-consent-deny")
                .small()
                .primary()
                .track_focus(section.deny_focus())
                .label("Deny")
                .on_click(cx.listener(|section, _, _, cx| section.deny(cx))),
        )
        .child(
            Button::new("user-consent-accept")
                .small()
                .outline()
                .label(if replaces { "Replace" } else { "Install" })
                .disabled(!consent.can_accept())
                .on_click(cx.listener(|section, _, window, cx| section.accept(window, cx))),
        )
        .into_any_element()
}

fn state(live: bool, cx: &Context<UserPluginsSection>) -> AnyElement {
    div()
        .id("user-consent-state")
        .aria_label(if live { "ready" } else { "inert" })
        .test_support()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(if live {
            "Check what it asks for, then decide."
        } else {
            "Read this first. The buttons wake up in a moment."
        })
        .into_any_element()
}

fn capability(
    consent: &Consent,
    line: &CapabilityLine,
    cx: &mut Context<UserPluginsSection>,
) -> AnyElement {
    let id = line.id.clone();
    let control = if line.required {
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child("Required")
            .into_any_element()
    } else {
        Switch::new(SharedString::from(format!("user-consent-allow:{id}")))
            .checked(consent.is_allowed(&id))
            .on_click(cx.listener(move |section, allowed: &bool, _, cx| {
                if let Some(consent) = section.consent_mut() {
                    consent.allow(&id, *allowed);
                }
                cx.notify();
            }))
            .into_any_element()
    };
    let description = if line.added {
        format!("New: {}", line.description)
    } else {
        line.description.clone()
    };
    let reason = line.reason.as_ref().map(|reason| {
        format!(
            "Author says (unverified): \u{201c}{}\u{201d}",
            quote(reason)
        )
    });
    div()
        .id(SharedString::from(format!(
            "user-consent-capability:{}",
            line.id
        )))
        .aria_label(match &reason {
            Some(reason) => format!("{description} | {reason}"),
            None => description.clone(),
        })
        .test_support()
        .child(form::row(
            description,
            reason.unwrap_or_default(),
            control,
            cx,
        ))
        .into_any_element()
}

fn field(line: &FieldLine, cx: &Context<UserPluginsSection>) -> AnyElement {
    div()
        .py_1()
        .text_sm()
        .when(line.added, |text| text.text_color(cx.theme().foreground))
        .child(if line.added {
            format!("New: {}", line.label)
        } else {
            line.label.clone()
        })
        .into_any_element()
}

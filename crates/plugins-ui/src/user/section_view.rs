//! How the User section looks: the install button, the consent, the plugin list and the detail
//! of the selected plugin.

use gpui_kit::{
    AnyElement, Context, IntoElement, KeyDownEvent, SharedString, TestSupportExt as _, Window,
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
use nocterm_plugin_model::PluginId;
use nocterm_ui::form;

use super::{
    consent_view::consent_dialog,
    port::{RowState, UserPluginRow, describe},
    removal::RemovalKind,
    section::UserPluginsSection,
    switch_view,
};

/// The most audit lines shown, newest first.
const AUDIT_SHOWN: usize = 20;

pub(super) fn view(
    section: &UserPluginsSection,
    _window: &mut Window,
    cx: &mut Context<UserPluginsSection>,
) -> impl IntoElement {
    let loaded = section.loaded();
    v_flex()
        .id("user-plugins")
        .aria_label("User plugins")
        .test_support()
        .w_full()
        .gap_2()
        .pt_6()
        .child(form::page_header(
            "User plugins",
            Some(
                "Plugins you install from a file. Their publishers are not verified, and they \
                 can only do what you allow here."
                    .into(),
            ),
            cx,
        ))
        .children(switch_view::switch_row(section, cx))
        .children(switch_view::explanation(section, cx))
        .when(loaded, |section_view| {
            section_view.child(
                div().child(
                    Button::new("user-install")
                        .small()
                        .outline()
                        .label("Install from file…")
                        .disabled(section.consent().is_some())
                        .on_click(cx.listener(|section, _, window, cx| {
                            section.choose_file(window, cx);
                        })),
                ),
            )
        })
        .children(dev_banner(section, cx))
        .children(section.notice().map(|notice| {
            div()
                .id("user-notice")
                .aria_label(notice.clone())
                .test_support()
                .child(form::error_text(notice.clone(), cx))
        }))
        .children(loaded.then(|| match section.consent() {
            Some(consent) => consent_dialog(section, consent, cx),
            None => listing(section, cx),
        }))
}

/// The banner of a developer session: the plugin and settings are not saved, and one button
/// loads the file under test through the same consent as any other package.
fn dev_banner(
    section: &UserPluginsSection,
    cx: &mut Context<UserPluginsSection>,
) -> Option<AnyElement> {
    let location = section.dev_location(cx)?;
    let name = std::path::Path::new(&location).file_name().map_or_else(
        || location.clone(),
        |name| name.to_string_lossy().into_owned(),
    );
    Some(
        v_flex()
            .id("user-dev")
            .aria_label("Developer session")
            .test_support()
            .w_full()
            .gap_1()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child("Developer session \u{2014} the plugin and settings are not saved"),
            )
            .child(
                div().child(
                    Button::new("user-dev-load")
                        .small()
                        .outline()
                        .label(format!("Load {name}"))
                        .disabled(section.consent().is_some())
                        .on_click(cx.listener(move |section, _, window, cx| {
                            section.begin_install(&location, window, cx);
                        })),
                ),
            )
            .into_any_element(),
    )
}

fn listing(section: &UserPluginsSection, cx: &mut Context<UserPluginsSection>) -> AnyElement {
    let rows = section.rows(cx);
    if rows.is_empty() {
        return form::note("No user plugins are installed.", cx);
    }
    let selected = section
        .selected()
        .and_then(|id| rows.iter().find(|row| &row.id == id))
        .cloned();
    let list: Vec<AnyElement> = rows
        .iter()
        .map(|row| plugin_row(section, row, cx))
        .collect();
    v_flex()
        .w_full()
        .gap_3()
        .child(
            v_flex()
                .id("user-plugin-list")
                .w_full()
                .gap_1()
                .children(list),
        )
        .children(selected.map(|row| detail(section, &row, cx)))
        .child(audit(section, cx))
        .into_any_element()
}

fn chip(plugin: &PluginId, kind: &'static str, cx: &Context<UserPluginsSection>) -> AnyElement {
    div()
        .id(SharedString::from(format!("user-chip:{plugin}:{kind}")))
        .aria_label(kind)
        .test_support()
        .px_1p5()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(kind)
        .into_any_element()
}

fn plugin_row(
    section: &UserPluginsSection,
    row: &UserPluginRow,
    cx: &mut Context<UserPluginsSection>,
) -> AnyElement {
    let id = row.id.clone();
    let selected = section.selected() == Some(&id);
    let theme = cx.theme();
    let (foreground, hover) = (theme.foreground, theme.foreground.opacity(0.05));
    let radius = theme.radius;
    let state = match &row.state {
        RowState::Ready => None,
        RowState::Faulted => Some("Faulted"),
        RowState::Broken(_) => Some("Broken"),
    };
    v_flex()
        .id(SharedString::from(format!("user-plugin-row:{id}")))
        .aria_label(row.name.clone())
        .test_support()
        .w_full()
        .gap_1()
        .px_3()
        .py_2()
        .rounded(radius)
        .cursor_pointer()
        .when(selected, |line| line.bg(foreground.opacity(0.08)))
        .hover(|line| line.bg(hover))
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().text_sm().font_medium().child(row.name.clone()))
                .when(!row.enabled, |line| line.child(chip(&id, "Disabled", cx)))
                .when(!row.placeable, |line| {
                    line.child(chip(&id, "Not placed", cx))
                })
                .children(state.map(|state| chip(&id, state, cx))),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{} \u{b7} publisher (unverified)", row.publisher)),
        )
        .on_click(cx.listener(move |section, _, window, cx| section.select(&id, window, cx)))
        .into_any_element()
}

pub(super) fn fact(
    label: &'static str,
    value: String,
    cx: &Context<UserPluginsSection>,
) -> AnyElement {
    h_flex()
        .gap_2()
        .text_sm()
        .child(
            div()
                .w_24()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(div().child(value))
        .into_any_element()
}

fn detail(
    section: &UserPluginsSection,
    row: &UserPluginRow,
    cx: &mut Context<UserPluginsSection>,
) -> AnyElement {
    let id = row.id.clone();
    let working = matches!(row.state, RowState::Ready | RowState::Faulted);
    let enabled = Switch::new("user-plugin-enabled")
        .checked(row.enabled)
        .on_click({
            let id = id.clone();
            cx.listener(move |section, checked: &bool, _, cx| {
                section.set_enabled(&id, *checked, cx);
            })
        });
    let placeable = Switch::new("user-plugin-placeable")
        .checked(row.placeable)
        .on_click({
            let id = id.clone();
            cx.listener(move |section, checked: &bool, _, cx| {
                section.set_placeable(&id, *checked, cx);
            })
        });
    let granted = if row.granted.is_empty() {
        "Nothing".to_owned()
    } else {
        row.granted.join("; ")
    };
    let broken = match &row.state {
        RowState::Broken(reason) => Some(form::error_text(describe(reason), cx)),
        _ => None,
    };
    let editor = match section.form() {
        Some(form) if working => form.clone().into_any_element(),
        _ => form::note("No settings", cx),
    };
    v_flex()
        .id("user-plugin-detail")
        .aria_label(row.name.clone())
        .test_support()
        .w_full()
        .gap_1()
        .child(fact("Name", row.name.clone(), cx))
        .child(fact("Id", row.id.to_string(), cx))
        .child(fact("Version", row.version.clone(), cx))
        .child(fact("Fingerprint", row.fingerprint.clone(), cx))
        .child(fact(
            "Publisher",
            format!("{} (unverified)", row.publisher),
            cx,
        ))
        .child(fact("Allowed to", granted, cx))
        .children(broken)
        .child(form::row(
            "Enabled",
            "Off hides its panels and widgets in every window and drops what it queued.",
            enabled,
            cx,
        ))
        .child(form::row(
            "Allowed in windows",
            "Revoking closes its panels at once. Its place in your layout is kept.",
            placeable,
            cx,
        ))
        .child(editor)
        .child(removal(section, row, cx))
        .into_any_element()
}

/// The three removal buttons, or the question once one of them was pressed.
fn removal(
    section: &UserPluginsSection,
    row: &UserPluginRow,
    cx: &mut Context<UserPluginsSection>,
) -> AnyElement {
    if let Some(pending) = section
        .pending_removal()
        .filter(|pending| *pending.target() == row.id)
    {
        return confirm_removal(pending.kind(), &row.name, section, cx);
    }
    let button =
        |button_id: &'static str, kind: RemovalKind, cx: &mut Context<UserPluginsSection>| {
            let id = row.id.clone();
            Button::new(button_id)
                .small()
                .outline()
                .label(kind.label())
                .on_click(cx.listener(move |section, _, window, cx| {
                    section.ask_removal(&id, kind, window, cx);
                }))
        };
    h_flex()
        .gap_2()
        .pt_2()
        .child(button("user-plugin-remove", RemovalKind::Keep, cx))
        .child(button("user-plugin-remove-all", RemovalKind::WithData, cx))
        .child(button("user-plugin-remove-data", RemovalKind::DataOnly, cx))
        .into_any_element()
}

/// What the removal takes away. The question holds focus, so Escape cancels; the named action
/// sits beside the primary Cancel.
fn confirm_removal(
    kind: RemovalKind,
    name: &str,
    section: &UserPluginsSection,
    cx: &mut Context<UserPluginsSection>,
) -> AnyElement {
    let question = kind.question(name);
    v_flex()
        .id("user-removal")
        .aria_label(question.clone())
        .test_support()
        .gap_1()
        .pt_2()
        .track_focus(section.deny_focus())
        .on_key_down(cx.listener(|section, event: &KeyDownEvent, _, cx| {
            if event.keystroke.key == "escape" {
                section.cancel_removal(cx);
            }
        }))
        .child(div().text_sm().child(question))
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("user-removal-cancel")
                        .small()
                        .primary()
                        .label("Cancel")
                        .on_click(cx.listener(|section, _, _, cx| section.cancel_removal(cx))),
                )
                .child(
                    Button::new("user-removal-confirm")
                        .small()
                        .outline()
                        .label(kind.label())
                        .on_click(cx.listener(|section, _, _, cx| section.confirm_removal(cx))),
                ),
        )
        .into_any_element()
}

fn audit(section: &UserPluginsSection, cx: &Context<UserPluginsSection>) -> AnyElement {
    let lines = section.audit(cx);
    let rows = lines
        .iter()
        .rev()
        .take(AUDIT_SHOWN)
        .enumerate()
        .map(|(n, line)| {
            div()
                .id(SharedString::from(format!("user-audit:{n}")))
                .aria_label(format!("{} {}", line.plugin, line.action))
                .test_support()
                .py_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{} \u{b7} {}", line.plugin, line.action))
                .into_any_element()
        });
    let rows: Vec<AnyElement> = rows.collect();
    if rows.is_empty() {
        return div().into_any_element();
    }
    form::section("Audit log", rows, cx)
}

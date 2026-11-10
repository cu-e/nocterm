//! How the User section looks: the head (the switch, install and the folder, the consent), the
//! rows of the installed plugins and the detail of the selected one. The Plugins page draws each
//! part in its place ([`UserPart`]); on its own the section draws them one under the other.

use gpui_kit::{
    AnyElement, Context, IntoElement, SharedString, TestSupportExt as _, Window,
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
use nocterm_ui::{IconName, form};

use super::{
    consent_view::consent_dialog,
    port::{RowState, UserPluginRow, describe},
    section::{FoldedList, UserPluginsSection},
    switch_view,
};

/// The most audit lines kept, newest first.
const AUDIT_SHOWN: usize = 20;
/// The lines a folded list of the detail shows.
const FOLDED_LINES: usize = 3;

/// A part of the section that the Plugins page draws in its own place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserPart {
    /// Above the plugin list: the switch, install and the plugins folder, notices, the consent.
    Head,
    /// In the plugin list: one row per installed plugin, each marked External.
    Rows,
    /// Beside the list: the selected plugin and the audit log.
    Detail,
}

pub(super) fn view(
    section: &UserPluginsSection,
    _window: &mut Window,
    cx: &mut Context<UserPluginsSection>,
) -> impl IntoElement {
    v_flex()
        .id("user-plugins")
        .aria_label("User plugins")
        .test_support()
        .w_full()
        .gap_3()
        .child(head(section, cx))
        .children(rows(section, cx))
        .children(selected_detail(section, cx))
}

/// One part of the section, for the Plugins page.
pub(super) fn part(
    section: &UserPluginsSection,
    part: UserPart,
    cx: &mut Context<UserPluginsSection>,
) -> AnyElement {
    let drawn = match part {
        UserPart::Head => Some(head(section, cx)),
        UserPart::Rows => rows(section, cx),
        UserPart::Detail => selected_detail(section, cx),
    };
    drawn.unwrap_or_else(|| div().into_any_element())
}

fn head(section: &UserPluginsSection, cx: &mut Context<UserPluginsSection>) -> AnyElement {
    let loaded = section.loaded();
    v_flex()
        .id("user-plugins-head")
        .test_support()
        .w_full()
        .gap_2()
        .child(form::page_header(
            "External plugins",
            Some(
                "Plugins you install from a file. Their publishers are not verified, and they \
                 can only do what you allow here."
                    .into(),
            ),
            cx,
        ))
        .children(switch_view::switch_row(section, cx))
        .children(switch_view::explanation(section, cx))
        .child(install_row(section, loaded, cx))
        .children(folder_listing(section, loaded, cx))
        .children(dev_banner(section, cx))
        .children(
            section
                .notice()
                .cloned()
                .or_else(|| section.manager_error(cx))
                .map(|notice| {
                    div()
                        .id("user-notice")
                        .aria_label(notice.clone())
                        .test_support()
                        .child(form::error_text(notice.clone(), cx))
                }),
        )
        .children(
            section
                .consent()
                .filter(|_| loaded)
                .map(|consent| consent_dialog(section, consent, cx)),
        )
        .into_any_element()
}

/// Install a plugin from a file and open the plugins folder. In a run that holds the plugins the
/// folder still opens, and the install button says what to do first.
fn install_row(
    section: &UserPluginsSection,
    loaded: bool,
    cx: &mut Context<UserPluginsSection>,
) -> AnyElement {
    let install = Button::new("user-install")
        .small()
        .outline()
        .label("Install plugin…")
        .disabled(!loaded || section.consent().is_some())
        .on_click(cx.listener(|section, _, window, cx| {
            section.choose_file(window, cx);
        }));
    v_flex()
        .gap_1()
        .child(
            h_flex()
                .id("user-install-row")
                .test_support()
                .gap_2()
                .child(install)
                .child(
                    Button::new("user-open-folder")
                        .small()
                        .ghost()
                        .label("Open plugins folder")
                        .on_click(cx.listener(|section, _, _, cx| section.open_folder(cx))),
                ),
        )
        .when(!loaded, |row| {
            row.child(
                div()
                    .id("user-install-held")
                    .aria_label(
                        "Allow external plugins above and restart Nocterm to install plugins.",
                    )
                    .test_support()
                    .child(form::note(
                        "Allow external plugins above and restart Nocterm to install plugins. \
                         Files you put in the plugins folder are offered here then.",
                        cx,
                    )),
            )
        })
        .children(
            section
                .folder()
                .error()
                .cloned()
                .map(|error| form::error_text(error, cx)),
        )
        .into_any_element()
}

/// The plugin files in the plugins folder, each with a button that installs it through the
/// consent. Shown only in a run that loads user plugins.
fn folder_listing(
    section: &UserPluginsSection,
    loaded: bool,
    cx: &mut Context<UserPluginsSection>,
) -> Option<AnyElement> {
    let found = section.folder().found();
    if !loaded || found.is_empty() || section.consent().is_some() {
        return None;
    }
    let rows = found.iter().enumerate().map(|(index, path)| {
        let name = path.file_name().map_or_else(
            || path.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let location = path.to_string_lossy().into_owned();
        h_flex()
            .id(("user-folder-file", index))
            .aria_label(SharedString::from(name.clone()))
            .test_support()
            .w_full()
            .gap_2()
            .child(div().flex_1().min_w_0().truncate().text_sm().child(name))
            .child(
                Button::new(("user-folder-install", index))
                    .small()
                    .outline()
                    .label("Install…")
                    .on_click(cx.listener(move |section, _, window, cx| {
                        section.begin_install(&location, window, cx);
                    })),
            )
            .into_any_element()
    });
    Some(
        v_flex()
            .id("user-folder")
            .aria_label("In your plugins folder")
            .test_support()
            .w_full()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child("In your plugins folder"),
            )
            .children(rows)
            .into_any_element(),
    )
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

/// The rows of the installed plugins, in a run that loads them.
fn rows(section: &UserPluginsSection, cx: &mut Context<UserPluginsSection>) -> Option<AnyElement> {
    if !section.loaded() {
        return None;
    }
    let rows = section.rows(cx);
    let list: Vec<AnyElement> = rows
        .iter()
        .map(|row| plugin_row(section, row, cx))
        .collect();
    Some(
        v_flex()
            .id("user-plugin-list")
            .test_support()
            .w_full()
            .gap_1()
            .when(list.is_empty(), |empty| {
                empty.child(form::note("No external plugins are installed.", cx))
            })
            .children(list)
            .into_any_element(),
    )
}

/// The selected plugin and the audit log, while one is selected.
fn selected_detail(
    section: &UserPluginsSection,
    cx: &mut Context<UserPluginsSection>,
) -> Option<AnyElement> {
    if !section.loaded() {
        return None;
    }
    let row = section
        .selected()
        .and_then(|id| section.rows(cx).into_iter().find(|row| &row.id == id))?;
    Some(
        v_flex()
            .w_full()
            .gap_3()
            .child(detail(section, &row, cx))
            .child(audit(section, cx))
            .into_any_element(),
    )
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
                .child(chip(&id, "External", cx))
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
    facts(label, vec![value], cx)
}

/// A label and its values, one per line; a long value wraps inside the detail.
fn facts(label: &'static str, values: Vec<String>, cx: &Context<UserPluginsSection>) -> AnyElement {
    let values: Vec<AnyElement> = values
        .into_iter()
        .map(|value| div().w_full().child(value).into_any_element())
        .collect();
    labelled(
        label,
        v_flex().gap_0p5().children(values).into_any_element(),
        cx,
    )
}

/// A label beside `content`.
fn labelled(
    label: &'static str,
    content: AnyElement,
    cx: &Context<UserPluginsSection>,
) -> AnyElement {
    h_flex()
        .w_full()
        .gap_2()
        .items_start()
        .text_sm()
        .child(
            div()
                .w_24()
                .flex_none()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(content))
        .into_any_element()
}

/// What the plugin is allowed to do, the first lines shown and the rest behind the fold.
fn granted(
    section: &UserPluginsSection,
    row: &UserPluginRow,
    cx: &mut Context<UserPluginsSection>,
) -> AnyElement {
    let lines: Vec<AnyElement> = if row.granted.is_empty() {
        vec!["Nothing".to_owned()]
    } else {
        row.granted.clone()
    }
    .into_iter()
    .map(|line| div().w_full().pb_0p5().child(line).into_any_element())
    .collect();
    let list = form::fold_list(
        "user-granted-fold",
        lines,
        FOLDED_LINES,
        section.is_unfolded(FoldedList::Granted),
        cx.listener(|section, _, _, cx| section.toggle_fold(FoldedList::Granted, cx)),
        cx,
    );
    labelled("Allowed to", list, cx)
}

fn detail(
    section: &UserPluginsSection,
    row: &UserPluginRow,
    cx: &mut Context<UserPluginsSection>,
) -> AnyElement {
    let working = matches!(row.state, RowState::Ready | RowState::Faulted);
    let remove = {
        let id = row.id.clone();
        Button::new("user-plugin-remove")
            .xsmall()
            .ghost()
            .icon(IconName::Trash)
            .label("Remove…")
            .on_click(cx.listener(move |section, _, window, cx| {
                section.ask_removal(&id, window, cx);
            }))
    };
    let enabled = Switch::new("user-plugin-enabled")
        .checked(row.enabled && row.placeable)
        .on_click({
            let row = row.clone();
            cx.listener(move |section, checked: &bool, _, cx| {
                section.set_on(&row, *checked, cx);
            })
        });
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
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_semibold()
                        .child(row.name.clone()),
                )
                .child(remove),
        )
        .child(fact("Id", row.id.to_string(), cx))
        .child(fact("Version", row.version.clone(), cx))
        .child(fact("Fingerprint", row.fingerprint.clone(), cx))
        .child(fact(
            "Publisher",
            format!("{} (unverified)", row.publisher),
            cx,
        ))
        .child(granted(section, row, cx))
        .children(broken)
        .child(form::row(
            "Enabled",
            "Off closes what it shows in every window and keeps its place. An updated plugin \
             starts off.",
            enabled,
            cx,
        ))
        .child(editor)
        .into_any_element()
}

/// The audit log, newest first: the first lines shown and the rest behind the fold.
fn audit(section: &UserPluginsSection, cx: &mut Context<UserPluginsSection>) -> AnyElement {
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
    let list = form::fold_list(
        "user-audit-fold",
        rows,
        FOLDED_LINES,
        section.is_unfolded(FoldedList::Audit),
        cx.listener(|section, _, _, cx| section.toggle_fold(FoldedList::Audit, cx)),
        cx,
    );
    form::section("Audit log", [list], cx)
}

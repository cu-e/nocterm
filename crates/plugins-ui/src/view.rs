//! How the page looks: the plugin list and the detail of the selected plugin.

use gpui_kit::{
    AnyElement, Context, IntoElement, SharedString, TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, button::Button, h_flex,
        switch::Switch, v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_plugin_model::{ContributionKind, PluginId, SystemContribution, SystemPlugin};
use nocterm_ui::{contribution_icon, form};
use nocterm_workspace::{OpenLayoutSettings, OpenPluginPage};

use crate::page::PluginsPage;

pub(crate) fn page(page: &PluginsPage, cx: &mut Context<PluginsPage>) -> impl IntoElement {
    let rows: Vec<AnyElement> = page
        .catalog()
        .plugins()
        .map(|plugin| row(page, plugin, cx))
        .collect();
    let selected = page
        .selected()
        .and_then(|id| page.catalog().plugin(id))
        .map(|plugin| detail(page, plugin, cx));
    v_flex()
        .id("plugins-page")
        .test_support()
        .track_focus(&page_focus(page, cx))
        .w_full()
        .child(system_plugins(page, rows, selected, cx))
        .children(user_section(page))
}

#[cfg(feature = "user")]
fn user_section(page: &PluginsPage) -> Option<gpui_kit::AnyView> {
    page.user().map(|user| user.clone().into())
}

#[cfg(not(feature = "user"))]
fn user_section(_: &PluginsPage) -> Option<gpui_kit::AnyView> {
    None
}

fn system_plugins(
    _page: &PluginsPage,
    rows: Vec<AnyElement>,
    selected: Option<AnyElement>,
    cx: &mut Context<PluginsPage>,
) -> impl IntoElement {
    h_flex()
        .id("system-plugins")
        .w_full()
        .gap_6()
        .items_start()
        .child(
            v_flex()
                .id("plugins-list")
                .test_support()
                .w(rems(18.))
                .flex_shrink_0()
                .gap_1()
                .children(rows),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .children(selected.or_else(|| Some(form::note("No plugins", cx)))),
        )
}

fn page_focus(page: &PluginsPage, cx: &Context<PluginsPage>) -> gpui_kit::FocusHandle {
    gpui_kit::Focusable::focus_handle(page, cx)
}

fn count(plugin: &SystemPlugin) -> String {
    match plugin.contributions.len() {
        1 => "1 contribution".to_owned(),
        many => format!("{many} contributions"),
    }
}

fn chip(plugin: &PluginId, kind: &'static str, cx: &Context<PluginsPage>) -> AnyElement {
    div()
        .id(SharedString::from(format!("plugin-chip:{plugin}:{kind}")))
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

fn row(
    page: &PluginsPage,
    plugin: &'static SystemPlugin,
    cx: &mut Context<PluginsPage>,
) -> AnyElement {
    let id = plugin.id.clone();
    let selected = page.selected() == Some(&id);
    let enabled = page.is_enabled(&id, cx);
    let theme = cx.theme();
    let (foreground, hover) = (theme.foreground, theme.foreground.opacity(0.05));
    v_flex()
        .id(SharedString::from(format!("plugin-row:{id}")))
        .aria_label(plugin.name)
        .test_support()
        .w_full()
        .gap_1()
        .px_3()
        .py_2()
        .rounded(theme.radius)
        .cursor_pointer()
        .when(selected, |row| row.bg(foreground.opacity(0.08)))
        .hover(|row| row.bg(hover))
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().text_sm().font_medium().child(plugin.name))
                .when(!enabled, |line| line.child(chip(&id, "Disabled", cx)))
                .when(plugin.required, |line| {
                    line.child(chip(&id, "Required", cx))
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(plugin.description),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(count(plugin)),
        )
        .on_click(cx.listener(move |page, _, window, cx| page.select(&id, window, cx)))
        .into_any_element()
}

fn detail(
    page: &PluginsPage,
    plugin: &'static SystemPlugin,
    cx: &mut Context<PluginsPage>,
) -> AnyElement {
    let id = plugin.id.clone();
    let enabled = page.is_enabled(&id, cx);
    let switch = Switch::new("plugin-enabled")
        .checked(enabled)
        .disabled(plugin.required)
        .on_click({
            let id = id.clone();
            cx.listener(move |page, checked: &bool, _, cx| {
                page.set_enabled(id.clone(), *checked, cx);
            })
        });
    let help = if plugin.required {
        "Required: this plugin cannot be switched off."
    } else {
        "Off hides its panels and widgets in every window."
    };
    let editor = match page.form() {
        Some(form) => form.clone().into_any_element(),
        None => form::note("No settings", cx),
    };
    v_flex()
        .id("plugin-detail")
        .aria_label(plugin.name)
        .test_support()
        .w_full()
        .child(form::page_header(
            plugin.name,
            Some(plugin.description.into()),
            cx,
        ))
        .child(form::row("Enabled", help, switch, cx))
        .children(page.refusal(&id).map(|refusal| {
            div()
                .id("plugin-refusal")
                .aria_label(refusal.clone())
                .test_support()
                .child(form::error_text(refusal, cx))
        }))
        .child(contributions(plugin, cx))
        .child(editor)
        .children(page_link(plugin, cx))
        .children(layout_link(plugin, cx))
        .into_any_element()
}

/// What switching the plugin off removes: each contribution with its icon, title and kind.
fn contributions(plugin: &'static SystemPlugin, cx: &Context<PluginsPage>) -> AnyElement {
    let rows = plugin
        .contributions
        .iter()
        .map(|contribution| contribution_row(contribution, cx));
    form::section("Contributions", rows, cx)
}

fn contribution_row(contribution: &SystemContribution, cx: &Context<PluginsPage>) -> AnyElement {
    let kind = match contribution.kind {
        ContributionKind::Panel => "Side panel",
        ContributionKind::Widget => "Status bar widget",
    };
    h_flex()
        .id(SharedString::from(format!(
            "plugin-contribution:{}",
            contribution.id
        )))
        .aria_label(contribution.title)
        .test_support()
        .py_2()
        .gap_2()
        .items_center()
        .text_sm()
        .child(gpui_kit::component::Icon::new(contribution_icon(contribution.icon)).small())
        .child(div().child(contribution.title))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(kind),
        )
        .into_any_element()
}

/// The way to the plugin's own Settings page, when it names one.
fn page_link(plugin: &'static SystemPlugin, cx: &mut Context<PluginsPage>) -> Option<AnyElement> {
    plugin.settings_page?;
    let id: SharedString = plugin.id.as_str().to_owned().into();
    Some(
        div()
            .pt_4()
            .child(
                Button::new("plugin-page-link")
                    .small()
                    .outline()
                    .label(format!("{} settings…", plugin.name))
                    .on_click(cx.listener(move |_, _, window, cx| {
                        let action = OpenPluginPage { plugin: id.clone() };
                        window.dispatch_action(Box::new(action), cx);
                    })),
            )
            .into_any_element(),
    )
}

/// The way to the status bar and panels editor, with the plugin's first contribution selected.
/// A plugin that contributes nothing has nothing to place.
fn layout_link(plugin: &'static SystemPlugin, cx: &mut Context<PluginsPage>) -> Option<AnyElement> {
    let first: SharedString = plugin.contributions.first()?.id.as_str().to_owned().into();
    Some(
        div()
            .pt_4()
            .child(
                Button::new("plugin-layout-link")
                    .small()
                    .outline()
                    .label("Customize in status bar…")
                    .on_click(cx.listener(move |_, _, window, cx| {
                        let action = OpenLayoutSettings {
                            select: Some(first.clone()),
                        };
                        window.dispatch_action(Box::new(action), cx);
                    })),
            )
            .into_any_element(),
    )
}

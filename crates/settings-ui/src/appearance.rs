//! Light/dark pickers and theme management in the native Appearance form.
use crate::{SettingsView, pages};
use gpui_kit::{
    AnyElement, App, Context, Entity, SharedString, Subscription, Window,
    component::{
        IndexPath, Sizable as _,
        select::{SearchableVec, Select, SelectEvent, SelectState},
    },
    prelude::*,
};
use nocterm_themes::Appearance;
use nocterm_ui::{AppearanceMode, UiLayout};
use nocterm_ui::{ThemeUi, form};
use std::collections::BTreeMap;
mod browser;
pub(crate) mod commands;
mod installed;
#[cfg(test)]
mod tests;
use browser::ThemeBrowser;

type Picker = Entity<SelectState<SearchableVec<String>>>;
pub(crate) struct AppearancePage {
    themes: ThemeUi,
    settings: nocterm_store::SettingsHandle,
    light: Picker,
    dark: Picker,
    browser: Entity<ThemeBrowser>,
    errors: BTreeMap<String, SharedString>,
    folder_error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}
impl AppearancePage {
    pub(crate) fn new(
        themes: ThemeUi,
        settings: nocterm_store::SettingsHandle,
        window: &mut Window,
        cx: &mut Context<SettingsView>,
    ) -> Self {
        let picker = |appearance, window: &mut Window, cx: &mut Context<SettingsView>| {
            let items = choices(&themes, appearance, cx);
            let selected = selection(&themes, &settings, appearance, cx);
            let index = items.iter().position(|name| name == &selected).unwrap_or(0);
            cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(items),
                    Some(IndexPath::new(index)),
                    window,
                    cx,
                )
                .searchable(true)
            })
        };
        let light = picker(Appearance::Light, window, cx);
        let dark = picker(Appearance::Dark, window, cx);
        let mut subscriptions = Vec::new();
        for (picker, appearance) in [(&light, Appearance::Light), (&dark, Appearance::Dark)] {
            let subscription = cx.subscribe(
                picker,
                move |this, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                    let SelectEvent::Confirm(name) = event;
                    let name = name.clone().filter(|s| s != "Nocterm Default");
                    this.save(
                        move |s: &mut nocterm_ui::AppearanceSettings| match appearance {
                            Appearance::Light => s.light_theme = name,
                            Appearance::Dark => s.dark_theme = name,
                        },
                        cx,
                    );
                },
            );
            subscriptions.push(subscription);
        }
        subscriptions.push(themes.themes().observe_in(window, cx, |this, window, cx| {
            this.appearance.sync(window, cx);
            cx.notify();
        }));
        let browser = cx.new(|cx| ThemeBrowser::new(themes.clone(), window, cx));
        Self {
            themes,
            settings,
            light,
            dark,
            browser,
            errors: BTreeMap::new(),
            folder_error: None,
            _subscriptions: subscriptions,
        }
    }
    pub(crate) fn sync(&mut self, window: &mut Window, cx: &mut Context<SettingsView>) {
        for (picker, appearance) in [
            (&self.light, Appearance::Light),
            (&self.dark, Appearance::Dark),
        ] {
            let items = choices(&self.themes, appearance, cx);
            let selected = selection(&self.themes, &self.settings, appearance, cx);
            picker.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(items), window, cx);
                state.set_selected_value(&selected, window, cx);
            });
        }
    }
}
fn selection(
    themes: &ThemeUi,
    settings: &nocterm_store::SettingsHandle,
    appearance: Appearance,
    cx: &App,
) -> String {
    let name = match appearance {
        Appearance::Light => {
            &settings
                .setting::<nocterm_ui::AppearanceSettings>(cx)
                .light_theme
        }
        Appearance::Dark => {
            &settings
                .setting::<nocterm_ui::AppearanceSettings>(cx)
                .dark_theme
        }
    };
    name.as_ref()
        .filter(|name| themes.themes().catalog(cx).find(name, appearance).is_some())
        .cloned()
        .unwrap_or("Nocterm Default".into())
}
fn choices(themes: &ThemeUi, appearance: Appearance, cx: &App) -> Vec<String> {
    std::iter::once("Nocterm Default".into())
        .chain(
            themes
                .themes()
                .catalog(cx)
                .entries()
                .iter()
                .filter(|e| e.appearance == appearance)
                .map(|e| e.name.clone()),
        )
        .collect()
}
/// How the window's regions are framed.
fn layout(
    view: &SettingsView,
    settings: &nocterm_ui::AppearanceSettings,
    cx: &mut Context<SettingsView>,
) -> AnyElement {
    let mut rows = vec![form::row(
        "Window layout",
        "Floating draws the sidebar, tabs and panels as rounded cards on a darker canvas.",
        pages::choices(
            "ui-layout",
            settings.layout,
            &[
                ("Floating", UiLayout::Floating),
                ("Classic", UiLayout::Classic),
            ],
            |s: &mut nocterm_ui::AppearanceSettings, layout| s.layout = layout,
            cx,
        ),
        cx,
    )];
    if settings.layout == UiLayout::Floating {
        rows.push(pages::short_row(
            view,
            "appearance.card_gap",
            "Card gap",
            "Space between cards and around the window, 0–24 pixels.",
            cx,
        ));
        rows.push(pages::short_row(
            view,
            "appearance.card_radius",
            "Card radius",
            "Corner radius of cards, 0–24 pixels.",
            cx,
        ));
    }
    form::section("Layout", rows, cx)
}

pub(crate) fn render(view: &mut SettingsView, cx: &mut Context<SettingsView>) -> Vec<AnyElement> {
    let settings = view
        .installed
        .settings()
        .setting::<nocterm_ui::AppearanceSettings>(cx)
        .clone();
    let mut sections = vec![form::section(
        "Theme",
        [
            form::row(
                "Color scheme",
                "Follow the system, or always use light or dark.",
                pages::choices(
                    "appearance",
                    settings.mode,
                    &[
                        ("System", AppearanceMode::System),
                        ("Light", AppearanceMode::Light),
                        ("Dark", AppearanceMode::Dark),
                    ],
                    |s: &mut nocterm_ui::AppearanceSettings, mode| s.mode = mode,
                    cx,
                ),
                cx,
            ),
            form::stacked_row(
                "Light theme",
                "Used in Light mode and when the system uses light appearance.",
                Select::new(&view.appearance.light).small(),
                None,
                cx,
            ),
            form::stacked_row(
                "Dark theme",
                "Used in Dark mode and when the system uses dark appearance.",
                Select::new(&view.appearance.dark).small(),
                None,
                cx,
            ),
        ],
        cx,
    )];
    sections.push(layout(view, &settings, cx));
    sections.push(installed::render(view, cx));
    sections.push(form::section(
        "Get themes from Zed",
        [view.appearance.browser.clone().into_any_element()],
        cx,
    ));
    sections.push(form::section("Servers",[form::row("Show server countries","Look up the public IP address of each server on connecting and show its flag. Private addresses and host names are never sent. A country chosen on the connection is always shown.",pages::toggle("detect-server-country",settings.detect_server_country,false,|s: &mut nocterm_ui::AppearanceSettings,on|s.detect_server_country=on,cx),cx)],cx));
    sections
}

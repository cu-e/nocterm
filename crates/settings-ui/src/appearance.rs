//! Light/dark pickers and theme management in the native Appearance form.
use crate::{SettingsView, pages};
use gpui_kit::{
    AnyElement, App, BorrowAppContext as _, Context, Entity, Global, SharedString, Subscription,
    Window,
    component::{
        IndexPath, Sizable as _,
        select::{SearchableVec, Select, SelectEvent, SelectState},
    },
    prelude::*,
};
use nocterm_settings::AppearanceMode;
use nocterm_themes::{Appearance, ThemeRegistry};
use nocterm_ui::{ActiveSettings as _, ActiveThemes as _, Themes, form};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
mod browser;
pub(crate) mod commands;
mod installed;
#[cfg(test)]
mod tests;
use browser::ThemeBrowser;

#[derive(Default)]
struct Operations(BTreeSet<String>);
impl Global for Operations {}
fn busy(id: &str, cx: &App) -> bool {
    cx.try_global::<Operations>()
        .is_some_and(|o| o.0.contains(id))
}
fn begin(id: String, cx: &mut App) {
    cx.update_global::<Operations, _>(|o, _| {
        o.0.insert(id);
    });
}
fn end(id: &str, cx: &mut App) {
    cx.update_global::<Operations, _>(|o, _| {
        o.0.remove(id);
    });
}

type Picker = Entity<SelectState<SearchableVec<String>>>;
/// The app provides its client without changing Settings registration APIs.
struct Registry(Arc<dyn ThemeRegistry>);
impl Global for Registry {}
pub fn init_theme_registry(registry: Arc<dyn ThemeRegistry>, cx: &mut App) {
    cx.set_global(Registry(registry));
}

pub(crate) struct AppearancePage {
    light: Picker,
    dark: Picker,
    browser: Entity<ThemeBrowser>,
    errors: BTreeMap<String, SharedString>,
    folder_error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}
impl AppearancePage {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<SettingsView>) -> Self {
        if !cx.has_global::<Operations>() {
            cx.set_global(Operations::default());
        }
        let picker = |appearance, window: &mut Window, cx: &mut Context<SettingsView>| {
            let items = choices(appearance, cx);
            let selected = selection(appearance, cx);
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
                        move |s| match appearance {
                            Appearance::Light => s.appearance.light_theme = name,
                            Appearance::Dark => s.appearance.dark_theme = name,
                        },
                        cx,
                    );
                },
            );
            subscriptions.push(subscription);
        }
        if cx.has_global::<Themes>() {
            let subscription = cx.observe_global_in::<Themes>(window, |this, window, cx| {
                this.appearance.sync(window, cx);
                cx.notify();
            });
            subscriptions.push(subscription);
        }
        subscriptions.push(cx.observe_global::<Operations>(|_, cx| cx.notify()));
        let registry = cx.try_global::<Registry>().map(|r| r.0.clone());
        let browser = cx.new(|cx| ThemeBrowser::new(registry, window, cx));
        Self {
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
            let items = choices(appearance, cx);
            let selected = selection(appearance, cx);
            picker.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(items), window, cx);
                state.set_selected_value(&selected, window, cx);
            });
        }
    }
}
fn selection(appearance: Appearance, cx: &App) -> String {
    let name = match appearance {
        Appearance::Light => &cx.settings().appearance.light_theme,
        Appearance::Dark => &cx.settings().appearance.dark_theme,
    };
    name.as_ref()
        .filter(|name| {
            cx.themes()
                .is_some_and(|t| t.catalog.find(name, appearance).is_some())
        })
        .cloned()
        .unwrap_or("Nocterm Default".into())
}
fn choices(appearance: Appearance, cx: &App) -> Vec<String> {
    std::iter::once("Nocterm Default".into())
        .chain(
            cx.themes()
                .into_iter()
                .flat_map(|t| t.catalog.entries())
                .filter(|e| e.appearance == appearance)
                .map(|e| e.name.clone()),
        )
        .collect()
}
pub(crate) fn render(view: &mut SettingsView, cx: &mut Context<SettingsView>) -> Vec<AnyElement> {
    let settings = cx.settings().clone();
    let mut sections = vec![form::section(
        "Theme",
        [
            form::row(
                "Color scheme",
                "Follow the system, or always use light or dark.",
                pages::choices(
                    "appearance",
                    settings.appearance.mode,
                    &[
                        ("System", AppearanceMode::System),
                        ("Light", AppearanceMode::Light),
                        ("Dark", AppearanceMode::Dark),
                    ],
                    |s, mode| s.appearance.mode = mode,
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
    sections.push(installed::render(view, cx));
    sections.push(form::section(
        "Get themes from Zed",
        [view.appearance.browser.clone().into_any_element()],
        cx,
    ));
    sections.push(form::section("Servers",[form::row("Show server countries","Look up the public IP address of each server on connecting and show its flag. Private addresses and host names are never sent. A country chosen on the connection is always shown.",pages::toggle("detect-server-country",settings.appearance.detect_server_country,false,|s,on|s.appearance.detect_server_country=on,cx),cx)],cx));
    sections
}

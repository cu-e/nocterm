//! Light/dark pickers and theme management in the native Appearance form.
use self::controls as pages;
use crate::{AppearanceMode, UiLayout};
use crate::{ThemeUi, form};
use gpui_kit::{
    AnyElement, App, Context, Entity, SharedString, Subscription, Window,
    component::{
        IndexPath, Sizable as _,
        select::{SearchableVec, Select, SelectEvent, SelectState},
    },
    prelude::*,
};
use nocterm_themes::Appearance;
use std::collections::BTreeMap;
mod browser;
pub(crate) mod commands;
mod controls;
mod installed;
#[cfg(test)]
mod tests;
use browser::ThemeBrowser;

type Picker = Entity<SelectState<SearchableVec<String>>>;
struct AppearanceSettingsPage {
    themes: ThemeUi,
    settings: nocterm_store::SettingsHandle,
    light: Picker,
    dark: Picker,
    browser: Entity<ThemeBrowser>,
    errors: BTreeMap<String, SharedString>,
    folder_error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
    fields: std::collections::BTreeMap<SharedString, crate::FieldPresentation>,
    focus: gpui_kit::FocusHandle,
    feedback: crate::SettingsPageFeedback,
}
impl AppearanceSettingsPage {
    pub(crate) fn new(
        themes: ThemeUi,
        context: crate::SettingsPageContext,
        window: &mut Window,
        cx: &mut Context<AppearanceSettingsPage>,
    ) -> Self {
        let settings = context.settings().clone();
        let picker = |appearance, window: &mut Window, cx: &mut Context<AppearanceSettingsPage>| {
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
                        move |s: &mut crate::AppearanceSettings| match appearance {
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
            this.sync(window, cx);
            cx.notify();
        }));
        let browser = cx.new(|cx| ThemeBrowser::new(themes.clone(), window, cx));
        let weak = cx.weak_entity();
        let original_window = window.window_handle();
        subscriptions.push(settings.observe(cx, move |cx| {
            let _ = original_window.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.sync(window, cx);
                    cx.notify();
                });
            });
        }));
        let mut this = Self {
            themes,
            settings,
            light,
            dark,
            browser,
            errors: BTreeMap::new(),
            folder_error: None,
            _subscriptions: subscriptions,
            fields: BTreeMap::new(),
            focus: cx.focus_handle(),
            feedback: context.feedback().clone(),
        };
        appearance_fields(&mut this, window, cx);
        this
    }
    pub(crate) fn sync(&mut self, window: &mut Window, cx: &mut Context<AppearanceSettingsPage>) {
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
                .setting::<crate::AppearanceSettings>(cx)
                .light_theme
        }
        Appearance::Dark => &settings.setting::<crate::AppearanceSettings>(cx).dark_theme,
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
    view: &AppearanceSettingsPage,
    settings: &crate::AppearanceSettings,
    cx: &mut Context<AppearanceSettingsPage>,
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
            |s: &mut crate::AppearanceSettings, layout| s.layout = layout,
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

fn render(
    view: &mut AppearanceSettingsPage,
    cx: &mut Context<AppearanceSettingsPage>,
) -> Vec<AnyElement> {
    let settings = &view
        .settings
        .setting::<crate::AppearanceSettings>(cx)
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
                    |s: &mut crate::AppearanceSettings, mode| s.mode = mode,
                    cx,
                ),
                cx,
            ),
            form::stacked_row(
                "Light theme",
                "Used in Light mode and when the system uses light appearance.",
                Select::new(&view.light).small(),
                None,
                cx,
            ),
            form::stacked_row(
                "Dark theme",
                "Used in Dark mode and when the system uses dark appearance.",
                Select::new(&view.dark).small(),
                None,
                cx,
            ),
        ],
        cx,
    )];
    sections.push(layout(view, settings, cx));
    sections.push(installed::render(view, cx));
    sections.push(form::section(
        "Get themes from Zed",
        [view.browser.clone().into_any_element()],
        cx,
    ));
    sections.push(form::section("Servers",[form::row("Show server countries","Look up the public IP address of each server on connecting and show its flag. Private addresses and host names are never sent. A country chosen on the connection is always shown.",pages::toggle("detect-server-country",settings.detect_server_country,false,|s: &mut crate::AppearanceSettings,on|s.detect_server_country=on,cx),cx)],cx));
    sections
}

use crate::{SettingsTextField, settings_input::SettingsInputError as FieldError};
use nocterm_settings::SettingsSection;
impl AppearanceSettingsPage {
    fn settings(&self) -> &nocterm_store::SettingsHandle {
        &self.settings
    }
    /// Adds a text setting of section `S` saved under `key`.
    pub(super) fn add_field<S: SettingsSection>(
        &mut self,
        key: impl Into<SharedString>,
        read: impl Fn(&S) -> String + 'static,
        write: impl Fn(&mut S, &str) -> Result<(), FieldError> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = key.into();
        let field = SettingsTextField::new(
            self.settings().clone(),
            key.clone(),
            move |document| read(document.get::<S>()),
            move |document, text| {
                let mut section = document.get::<S>().clone();
                write(&mut section, text).map_err(nocterm_store::StoreError::edit)?;
                document.set(section);
                Ok(())
            },
            window,
            cx,
        );
        self.fields
            .insert(key, crate::FieldPresentation::new(field, window, cx));
    }
    pub(super) fn field(&self, key: &str) -> &SettingsTextField {
        self.fields[key].field()
    }

    pub(super) fn save<S: SettingsSection>(
        &mut self,
        edit: impl FnOnce(&mut S) + 'static,
        cx: &mut Context<Self>,
    ) {
        let task = self.settings().update::<S>(cx, edit);
        self.feedback.report(task, cx);
    }
}

fn appearance_fields(
    view: &mut AppearanceSettingsPage,
    window: &mut Window,
    cx: &mut Context<AppearanceSettingsPage>,
) {
    view.add_field(
        "appearance.card_gap",
        |s: &AppearanceSettings| s.card_gap.to_string(),
        |s: &mut AppearanceSettings, text| {
            s.card_gap = number(text, "Card gap", CARD_GAP_RANGE)?;
            Ok(())
        },
        window,
        cx,
    );
    view.add_field(
        "appearance.card_radius",
        |s: &AppearanceSettings| s.card_radius.to_string(),
        |s: &mut AppearanceSettings, text| {
            s.card_radius = number(text, "Card radius", CARD_RADIUS_RANGE)?;
            Ok(())
        },
        window,
        cx,
    );
}

use crate::{AppearanceSettings, CARD_GAP_RANGE, CARD_RADIUS_RANGE, settings_input::number};

pub fn appearance_settings_page(themes: crate::ThemeUi) -> crate::SettingsPageSpec {
    crate::SettingsPageSpec::contextual("appearance", "Appearance", move |context, window, cx| {
        cx.new(|cx| AppearanceSettingsPage::new(themes.clone(), context, window, cx))
    })
    .with_icon(crate::IconName::Palette)
}
impl gpui_kit::Focusable for AppearanceSettingsPage {
    fn focus_handle(&self, _: &App) -> gpui_kit::FocusHandle {
        self.focus.clone()
    }
}
impl crate::SettingsPage for AppearanceSettingsPage {
    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for field in self.fields.values() {
            field.field().flush(window, cx);
        }
    }
}
impl gpui_kit::Render for AppearanceSettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui_kit::IntoElement {
        let sections = render(self, cx);
        gpui_kit::component::v_flex()
            .w_full()
            .child(form::page_header("Appearance", None, cx))
            .children(sections)
    }
}

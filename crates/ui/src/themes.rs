//! Explicit shared theme projection, independent of Settings UI.
use crate::Design;
use gpui_kit::{App, Subscription, component::ThemeColor};
use nocterm_design::DesignTokens;
use nocterm_store::SettingsHandle;
use nocterm_theme_service::ThemesHandle;
use nocterm_themes::Appearance;
use std::rc::Rc;
#[derive(Clone)]
pub struct ThemeUi(Rc<Installed>);
struct Installed {
    themes: ThemesHandle,
    settings: SettingsHandle,
    _subscriptions: Vec<Subscription>,
}
impl ThemeUi {
    pub fn install(
        settings: SettingsHandle,
        themes: ThemesHandle,
        base: DesignTokens,
        cx: &mut App,
    ) -> Self {
        settings.register::<crate::AppearanceSettings>(cx);
        settings.register::<crate::TerminalSettings>(cx);
        resolve(&themes, &settings, &base, cx);
        apply(&settings, cx);
        let theme_projection = themes.clone();
        let settings_projection = settings.clone();
        let theme_base = base.clone();
        let theme_observer = themes.observe(cx, move |cx| {
            resolve(&theme_projection, &settings_projection, &theme_base, cx);
        });
        let theme_projection = themes.clone();
        let settings_projection = settings.clone();
        let setting_observer = settings.observe(cx, move |cx| {
            if !resolve(&theme_projection, &settings_projection, &base, cx) {
                apply(&settings_projection, cx);
            }
        });
        let paint_settings = settings.clone();
        let design_observer = cx.observe_global::<Design>(move |cx| {
            apply(&paint_settings, cx);
        });
        Self(Rc::new(Installed {
            themes,
            settings,
            _subscriptions: vec![theme_observer, setting_observer, design_observer],
        }))
    }
    /// Repaints using this installation’s published policy and current system appearance.
    pub fn apply(&self, cx: &mut App) {
        apply(self.settings(), cx);
    }
    pub fn themes(&self) -> &ThemesHandle {
        &self.0.themes
    }
    pub fn settings(&self) -> &SettingsHandle {
        &self.0.settings
    }
}
fn apply(settings: &SettingsHandle, cx: &mut App) {
    let appearance = settings.setting::<crate::AppearanceSettings>(cx).clone();
    crate::theme::apply_theme_for(&appearance, cx);
}
fn resolve(
    themes: &ThemesHandle,
    settings: &SettingsHandle,
    base: &DesignTokens,
    cx: &mut App,
) -> bool {
    let catalog = themes.catalog(cx);
    let design = cx.global::<Design>();
    let mut tokens = base.clone();
    let mut imported = [false; 2];
    let builtin = DesignTokens::builtin();
    let appearance = settings.setting::<crate::AppearanceSettings>(cx);
    for (appearance, name) in [
        (Appearance::Light, &appearance.light_theme),
        (Appearance::Dark, &appearance.dark_theme),
    ] {
        let Some(name) = name else {
            continue;
        };
        let dark = appearance.is_dark();
        if let Some(entry) = catalog.find(name, appearance) {
            let mut fallback = builtin.palette(dark).clone();
            // Imported component colours refine from ThemeColor, not from the
            // built-in terminal background or the user's theme.toml overrides.
            let component = if dark {
                ThemeColor::dark()
            } else {
                ThemeColor::light()
            };
            fallback.terminal.background = Some(crate::terminal_style::color(component.background));
            let palette = entry.theme.palette(&fallback);
            if dark {
                tokens.dark = palette;
            } else {
                tokens.light = palette;
            }
            imported[usize::from(dark)] = true;
        } else {
            tracing::warn!(%name, ?appearance, "selected theme is unavailable; using Nocterm Default");
        }
    }
    if design.tokens != tokens || design.imported != imported {
        cx.set_global(Design {
            base: base.clone(),
            tokens,
            imported,
        });
        true
    } else {
        false
    }
}
#[cfg(test)]
mod tests;

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
        resolve(&themes, &settings, &base, cx);
        crate::apply_theme(cx);
        let theme_projection = themes.clone();
        let settings_projection = settings.clone();
        let theme_base = base.clone();
        let theme_observer = themes.observe(cx, move |cx| {
            resolve(&theme_projection, &settings_projection, &theme_base, cx)
        });
        let theme_projection = themes.clone();
        let settings_projection = settings.clone();
        let setting_observer = settings.observe(cx, move |cx| {
            resolve(&theme_projection, &settings_projection, &base, cx)
        });
        Self(Rc::new(Installed {
            themes,
            settings,
            _subscriptions: vec![theme_observer, setting_observer],
        }))
    }
    pub fn themes(&self) -> &ThemesHandle {
        &self.0.themes
    }
    pub fn settings(&self) -> &SettingsHandle {
        &self.0.settings
    }
}
fn resolve(themes: &ThemesHandle, settings: &SettingsHandle, base: &DesignTokens, cx: &mut App) {
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
    }
}
#[cfg(test)]
mod tests;

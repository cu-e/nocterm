//! Projects the design tokens onto the component library's theme.

use std::rc::Rc;

use gpui_kit::{
    App, WindowAppearance,
    component::{
        Theme, ThemeConfig, ThemeConfigColors, ThemeMode, ThemeRegistry, button::ButtonMetrics,
    },
    px,
};
use nocterm_design::{Color, DesignTokens};
use nocterm_settings::AppearanceMode;
use serde_json::Value;

use crate::{ActiveDesign, ActiveSettings};

/// Rebuilds the component theme from the design tokens and the user's
/// appearance setting, and repaints every window.
pub fn apply_theme(cx: &mut App) {
    let tokens = cx.design().clone();
    let rem = tokens.typography.ui_size.unwrap_or(16.0);
    cx.set_global(ButtonMetrics {
        font_size: px(tokens.typography.button_size),
        height: px(tokens.layout.button_height * rem),
        padding: px(tokens.layout.button_padding * rem),
    });
    let dark = match cx.settings().appearance.mode {
        AppearanceMode::System => matches!(
            cx.window_appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
        AppearanceMode::Light => false,
        AppearanceMode::Dark => true,
    };

    let registry = ThemeRegistry::global(cx);
    let light = Rc::new(theme_config(registry.default_light_theme(), &tokens, false));
    let dark_config = Rc::new(theme_config(registry.default_dark_theme(), &tokens, true));

    Theme::update(cx, |theme| {
        theme.light_theme = light;
        theme.dark_theme = dark_config;
    });
    let mode = if dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    Theme::change(mode, None, cx);
    cx.refresh_windows();
}

/// The standard theme `base` with the tokens layered over it.
fn theme_config(base: &ThemeConfig, tokens: &DesignTokens, dark: bool) -> ThemeConfig {
    let mut config = base.clone();
    config.name = format!("{} {}", tokens.name, if dark { "Dark" } else { "Light" }).into();

    let typography = &tokens.typography;
    if let Some(family) = &typography.ui_font {
        config.font_family = Some(family.clone().into());
    }
    if let Some(size) = typography.ui_size {
        config.font_size = Some(size);
    }
    if let Some(family) = &typography.mono_font {
        config.mono_font_family = Some(family.clone().into());
    }
    if let Some(size) = typography.mono_size {
        config.mono_font_size = Some(size);
    }

    let shape = &tokens.shape;
    if let Some(radius) = shape.radius {
        config.radius = Some(radius.round() as usize);
    }
    if let Some(radius) = shape.radius_lg {
        config.radius_lg = Some(radius.round() as usize);
    }
    if let Some(shadow) = shape.shadow {
        config.shadow = Some(shadow);
    }

    config.colors = overlay_colors(&config.colors, tokens.palette(dark).ui.iter());
    config
}

/// `colors` with the named colours replaced. Names the theme does not have
/// are skipped; [`unknown_color_names`] reports them.
fn overlay_colors<'a>(
    colors: &ThemeConfigColors,
    overrides: impl Iterator<Item = (&'a str, Color)>,
) -> ThemeConfigColors {
    let Ok(Value::Object(mut fields)) = serde_json::to_value(colors) else {
        return colors.clone();
    };
    for (name, color) in overrides {
        if let Some(field) = fields.get_mut(name) {
            *field = Value::String(color.to_string());
        }
    }
    serde_json::from_value(Value::Object(fields)).unwrap_or_else(|error| {
        tracing::error!(%error, "could not apply the design token colours");
        colors.clone()
    })
}

/// Interface colours in `tokens` that the component theme has no slot for:
/// typos, or names from another version of the component library.
pub fn unknown_color_names(tokens: &DesignTokens) -> Vec<String> {
    let schema = schemars::schema_for!(ThemeConfigColors);
    let known = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    [&tokens.light, &tokens.dark]
        .into_iter()
        .flat_map(|palette| palette.ui.iter())
        .map(|(name, _)| name)
        .filter(|name| !known.contains_key(*name))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtin_tokens_name_only_known_colours() {
        assert!(unknown_color_names(&DesignTokens::builtin()).is_empty());
    }

    #[test]
    fn misspelt_colours_are_reported() {
        let tokens = DesignTokens::with_overrides(
            "[dark.ui]\nbackground = \"#000000\"\n\"sidebar.backgruond\" = \"#111111\"\n",
        )
        .unwrap();

        assert_eq!(unknown_color_names(&tokens), ["sidebar.backgruond"]);
    }

    #[test]
    fn token_colours_replace_the_standard_ones() {
        let tokens = DesignTokens::with_overrides(
            "[dark.ui]\nbackground = \"#101214\"\n\"primary.background\" = \"#3366ffcc\"\n",
        )
        .unwrap();

        let config = theme_config(&ThemeConfig::default(), &tokens, true);

        assert_eq!(config.colors.background.as_deref(), Some("#101214"));
        assert_eq!(config.colors.primary.as_deref(), Some("#3366ffcc"));
        assert_eq!(config.name.as_ref(), "Nocterm Default Dark");
    }

    #[test]
    fn unset_tokens_keep_the_standard_values() {
        let base = ThemeConfig {
            radius: Some(6),
            font_size: Some(16.0),
            ..ThemeConfig::default()
        };

        let config = theme_config(&base, &DesignTokens::builtin(), false);

        assert_eq!(config.radius, Some(6));
        assert_eq!(config.font_size, Some(16.0));
        assert_eq!(config.colors.background, None);
    }
}

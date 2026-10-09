//! Catalogue publication and selection resolution, independent of Settings UI.
use crate::{Design, SettingsExt, SettingsStore};
use gpui_kit::{App, BorrowAppContext as _, Global, Task, component::ThemeColor};
use nocterm_design::DesignTokens;
use nocterm_themes::{Appearance, ThemeCatalog, ThemeDirs};
use std::sync::Arc;

pub struct Themes {
    pub dirs: ThemeDirs,
    pub catalog: Arc<ThemeCatalog>,
    generation: u64,
}
impl Global for Themes {}
pub trait ActiveThemes {
    fn themes(&self) -> Option<&Themes>;
}
impl ActiveThemes for App {
    fn themes(&self) -> Option<&Themes> {
        self.try_global::<Themes>()
    }
}
/// Install a preloaded catalogue; performs no network requests.
pub fn init_themes(dirs: ThemeDirs, catalog: ThemeCatalog, cx: &mut App) {
    cx.set_global(Themes {
        dirs,
        catalog: Arc::new(catalog),
        generation: 0,
    });
    resolve(cx);
    crate::apply_theme(cx);
    cx.observe_global::<SettingsStore>(resolve).detach();
    cx.observe_global::<Themes>(resolve).detach();
}
fn resolve(cx: &mut App) {
    let Some(themes) = cx.try_global::<Themes>() else {
        return;
    };
    let design = cx.global::<Design>();
    let mut tokens = design.base.clone();
    let mut imported = [false; 2];
    let builtin = DesignTokens::builtin();
    let appearance = cx.setting::<crate::AppearanceSettings>();
    for (appearance, name) in [
        (Appearance::Light, &appearance.light_theme),
        (Appearance::Dark, &appearance.dark_theme),
    ] {
        let Some(name) = name else {
            continue;
        };
        let dark = appearance.is_dark();
        if let Some(entry) = themes.catalog.find(name, appearance) {
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
            base: design.base.clone(),
            tokens,
            imported,
        });
    }
}
/// Background rescan with a generation guard: an older scan cannot overwrite a newer one.
pub fn reload_themes(cx: &mut App) -> Task<()> {
    let Some(themes) = cx.try_global::<Themes>() else {
        return Task::ready(());
    };
    let dirs = themes.dirs.clone();
    let generation = themes.generation + 1;
    cx.update_global::<Themes, _>(|themes, _| themes.generation = generation);
    let scan = cx
        .background_executor()
        .spawn(async move { ThemeCatalog::load(&dirs) });
    cx.spawn(async move |cx| {
        let catalog = scan.await;
        cx.update(|cx| {
            if cx.global::<Themes>().generation == generation {
                cx.update_global::<Themes, _>(|themes, _| themes.catalog = Arc::new(catalog));
            }
        });
    })
}
#[cfg(test)]
mod tests;

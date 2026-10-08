//! Managed packs, original user files and catalogue diagnostics.
use crate::SettingsView;
use gpui_kit::{
    AnyElement, Context,
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    prelude::*,
};
use nocterm_themes::{Appearance, ThemeCatalog, ThemeSource, uninstall};
use nocterm_ui::{ActiveThemes as _, Themes, form, reload_themes};
use std::{collections::BTreeSet, fs};

pub(super) fn remove(view: &mut SettingsView, id: String, cx: &mut Context<SettingsView>) {
    if super::busy(&id, cx) {
        return;
    }
    let Some(themes) = cx.themes() else {
        return;
    };
    let dirs = themes.dirs.clone();
    let names: BTreeSet<_> = themes
        .catalog
        .entries()
        .iter()
        .filter(|e| matches!(&e.source,ThemeSource::Installed{id:pack,..} if pack==&id))
        .map(|e| e.name.clone())
        .collect();
    super::begin(id.clone(), cx);
    let removed = id.clone();
    view.appearance.errors.remove(&id);
    let task = cx.background_executor().spawn(async move {
        uninstall(&dirs.installed, &removed)?;
        Ok::<_, nocterm_themes::ThemeError>(ThemeCatalog::load(&dirs))
    });
    cx.spawn(async move |this, cx| {
        match task.await {
            Ok(catalog) => {
                let light: BTreeSet<_> = catalog
                    .entries()
                    .iter()
                    .filter(|e| e.appearance == Appearance::Light)
                    .map(|e| e.name.clone())
                    .collect();
                let dark: BTreeSet<_> = catalog
                    .entries()
                    .iter()
                    .filter(|e| e.appearance == Appearance::Dark)
                    .map(|e| e.name.clone())
                    .collect();
                let save = cx.update(|cx| {
                    nocterm_ui::edit_settings(cx, move |s| {
                        if s.appearance
                            .light_theme
                            .as_ref()
                            .is_some_and(|n| names.contains(n) && !light.contains(n))
                        {
                            s.appearance.light_theme = None;
                        }
                        if s.appearance
                            .dark_theme
                            .as_ref()
                            .is_some_and(|n| names.contains(n) && !dark.contains(n))
                        {
                            s.appearance.dark_theme = None;
                        }
                    })
                });
                if let Err(error) = save.await {
                    let _ = this.update(cx, |this, cx| {
                        this.appearance.errors.insert(
                            id.clone(),
                            format!("Removed theme, but could not save selection: {error}").into(),
                        );
                        cx.notify();
                    });
                }
                cx.update(reload_themes).await;
            }
            Err(error) => {
                let _ = this.update(cx, |this, cx| {
                    this.appearance
                        .errors
                        .insert(id.clone(), error.to_string().into());
                    cx.notify();
                });
            }
        }
        cx.update(|cx| super::end(&id, cx));
    })
    .detach();
    cx.notify();
}
#[expect(clippy::too_many_lines, reason = "predates the limit")]
pub(super) fn render(view: &SettingsView, cx: &mut Context<SettingsView>) -> AnyElement {
    let Some(themes) = cx.themes() else {
        return form::section(
            "Installed themes",
            [form::note(
                "Theme folders are unavailable in this window.",
                cx,
            )],
            cx,
        );
    };
    let catalog = themes.catalog.clone();
    let mut rows = vec![form::row(
        "Theme files",
        "Drop Zed JSON files in the themes folder, then Reload. Color overrides in theme.toml apply to Nocterm Default; typography and layout always apply.",
        h_flex()
            .gap_2()
            .child(
                Button::new("reload-themes")
                    .small()
                    .ghost()
                    .label("Reload")
                    .on_click(cx.listener(|_, _, _, cx| reload_themes(cx).detach())),
            )
            .child(
                Button::new("open-themes-folder")
                    .small()
                    .ghost()
                    .label("Open folder")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let path = cx.global::<Themes>().dirs.user.clone();
                        match fs::create_dir_all(&path) {
                            Ok(()) => {
                                this.appearance.folder_error = None;
                                cx.open_with_system(&path);
                            }
                            Err(e) => {
                                this.appearance.folder_error =
                                    Some(format!("Could not open themes folder: {e}").into())
                            }
                        };
                        cx.notify();
                    })),
            ),
        cx,
    )];
    if let Some(error) = view.appearance.folder_error.clone() {
        rows.push(form::error_text(error, cx));
    }
    for pack in catalog.packs() {
        let id = pack.id.clone();
        let busy = super::busy(&id, cx);
        let row = form::row(
            pack.name.clone(),
            format!("{} · {}", pack.version, pack.authors.join(", ")),
            Button::new(gpui_kit::SharedString::from(format!(
                "uninstall-theme-{id}"
            )))
            .small()
            .ghost()
            .label(if busy { "Working…" } else { "Uninstall" })
            .disabled(busy)
            .on_click(cx.listener(move |this, _, _, cx| remove(this, id.clone(), cx))),
            cx,
        );
        rows.push(
            v_flex()
                .child(row)
                .when_some(
                    view.appearance.errors.get(&pack.id).cloned(),
                    |el, error| el.child(form::error_text(error, cx)),
                )
                .into_any_element(),
        );
    }
    for (id, error) in &view.appearance.errors {
        if !catalog.packs().iter().any(|pack| &pack.id == id) {
            rows.push(form::error_text(error.clone(), cx));
        }
    }
    let files: BTreeSet<_> = catalog
        .entries()
        .iter()
        .filter_map(|e| match &e.source {
            ThemeSource::User(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    for file in files {
        rows.push(form::note(
            format!(
                "User file: {}",
                file.file_name().unwrap_or_default().to_string_lossy()
            ),
            cx,
        ));
    }
    if catalog.packs().is_empty() && catalog.entries().is_empty() {
        rows.push(form::note(
            "No themes installed. Browse Zed below or add a JSON file to the themes folder.",
            cx,
        ));
    }
    for problem in catalog.problems() {
        rows.push(form::error_text(
            format!("{}: {}", problem.file.display(), problem.error),
            cx,
        ));
    }
    form::section("Installed themes", rows, cx)
}

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
use nocterm_themes::{Appearance, ThemeSource};
use nocterm_ui::form;
use std::collections::BTreeSet;

pub(super) fn remove(view: &mut SettingsView, id: String, cx: &mut Context<SettingsView>) {
    let themes = view.appearance.themes.clone();
    let settings = view.appearance.settings.clone();
    let task = themes.themes().uninstall(id.clone(), cx);
    cx.spawn(async move |this, cx| match task.await {
        Ok(change) => {
            let save = cx.update(|cx| {
                settings.update::<nocterm_ui::AppearanceSettings>(cx, move |settings| {
                    for (slot, appearance) in [
                        (&mut settings.light_theme, Appearance::Light),
                        (&mut settings.dark_theme, Appearance::Dark),
                    ] {
                        if slot.as_ref().is_some_and(|name| {
                            change.removed.contains(&(name.clone(), appearance))
                                && change.catalog.find(name, appearance).is_none()
                        }) {
                            *slot = None;
                        }
                    }
                })
            });
            if let Err(error) = save.await {
                let _ = this.update(cx, |this, cx| {
                    this.appearance.errors.insert(
                        id,
                        format!("Removed theme, but could not save selection: {error}").into(),
                    );
                    cx.notify();
                });
            }
        }
        Err(error) => {
            let _ = this.update(cx, |this, cx| {
                this.appearance.errors.insert(id, error.to_string().into());
                cx.notify();
            });
        }
    })
    .detach();
}
fn open_folder(view: &mut SettingsView, cx: &mut Context<SettingsView>) {
    let Some(library) = view.appearance.themes.themes().library(cx) else {
        return;
    };
    let task = cx
        .background_executor()
        .spawn(async move { library.prepare_user_directory() });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        let _ = this.update(cx, |this, cx| {
            match result {
                Ok(path) => {
                    this.appearance.folder_error = None;
                    cx.open_with_system(&path);
                }
                Err(error) => {
                    this.appearance.folder_error =
                        Some(format!("Could not open themes folder: {error}").into())
                }
            }
            cx.notify();
        });
    })
    .detach();
}
pub(super) fn render(view: &SettingsView, cx: &mut Context<SettingsView>) -> AnyElement {
    let catalog = view.appearance.themes.themes().catalog(cx);
    let mut errors: std::collections::BTreeMap<String, gpui_kit::SharedString> = view
        .appearance
        .themes
        .themes()
        .errors(cx)
        .into_iter()
        .map(|(id, error)| (id, error.to_string().into()))
        .collect();
    errors.extend(view.appearance.errors.clone());
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
                    .on_click(
                        cx.listener(|this, _, _, cx| this.appearance.themes.themes().reload(cx)),
                    ),
            )
            .child(
                Button::new("open-themes-folder")
                    .small()
                    .ghost()
                    .label("Open folder")
                    .on_click(cx.listener(|this, _, _, cx| open_folder(this, cx))),
            ),
        cx,
    )];
    if let Some(error) = view.appearance.folder_error.clone() {
        rows.push(form::error_text(error, cx));
    }
    for pack in catalog.packs() {
        let id = pack.id.clone();
        let busy = view.appearance.themes.themes().busy(&id, cx);
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
                .when_some(errors.get(&pack.id).cloned(), |el, error| {
                    el.child(form::error_text(error, cx))
                })
                .into_any_element(),
        );
    }
    for (id, error) in &errors {
        if !catalog.packs().iter().any(|pack| &pack.id == id) {
            rows.push(form::error_text(error.clone(), cx));
        }
    }
    append_diagnostics(&catalog, &mut rows, cx);
    form::section("Installed themes", rows, cx)
}

fn append_diagnostics(
    catalog: &nocterm_themes::ThemeCatalog,
    rows: &mut Vec<AnyElement>,
    cx: &mut Context<SettingsView>,
) {
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
}

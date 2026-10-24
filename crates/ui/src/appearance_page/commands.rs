//! Searchable appearance pickers reached from the workspace command palette.
use crate::notice;
use crate::{AppearanceMode, PickerOption, open_choice_picker};
use gpui_kit::{App, Window, component::ActiveTheme as _};

use super::{choices, selection};

#[derive(Clone)]
enum Choice {
    Theme { dark: bool, name: String },
    Scheme(AppearanceMode),
}

pub fn open_theme_picker(
    themes: &crate::ThemeUi,
    settings: &nocterm_store::SettingsHandle,
    window: &mut Window,
    cx: &mut App,
) {
    // Snapshot the appearance at open: an OS change in System mode while the
    // picker is open must not redirect a light selection into the dark slot.
    let dark = cx.theme().is_dark();
    let appearance = if dark {
        nocterm_themes::Appearance::Dark
    } else {
        nocterm_themes::Appearance::Light
    };
    let selected = selection(themes, settings, appearance, cx);
    let options = choices(themes, appearance, cx)
        .into_iter()
        .map(|name| {
            let checked = name == selected;
            let choice = Choice::Theme {
                dark,
                name: name.clone(),
            };
            PickerOption::new(name, checked, choice)
        })
        .collect();
    let settings = settings.clone();
    open_choice_picker(
        if dark {
            "Choose dark theme…"
        } else {
            "Choose light theme…"
        },
        options,
        move |choice, window, cx| save(choice, &settings, window, cx),
        window,
        cx,
    );
}

pub fn open_scheme_picker(
    settings: &nocterm_store::SettingsHandle,
    window: &mut Window,
    cx: &mut App,
) {
    let current = settings.setting::<crate::AppearanceSettings>(cx).mode;
    let options = [
        ("System", AppearanceMode::System),
        ("Light", AppearanceMode::Light),
        ("Dark", AppearanceMode::Dark),
    ]
    .into_iter()
    .map(|(label, mode)| PickerOption::new(label, mode == current, Choice::Scheme(mode)))
    .collect();
    let settings = settings.clone();
    open_choice_picker(
        "Choose color scheme…",
        options,
        move |choice, window, cx| save(choice, &settings, window, cx),
        window,
        cx,
    );
}

fn save(
    choice: Choice,
    settings: &nocterm_store::SettingsHandle,
    window: &mut Window,
    cx: &mut App,
) {
    let task = settings.update::<crate::AppearanceSettings>(cx, move |settings| match choice {
        Choice::Theme { dark, name } => {
            let name = (name != "Nocterm Default").then_some(name);
            if dark {
                settings.dark_theme = name;
            } else {
                settings.light_theme = name;
            }
        }
        Choice::Scheme(mode) => settings.mode = mode,
    });
    window
        .spawn(cx, async move |cx| {
            let result = task.await;
            let _ = cx.update(|window, cx| match result {
                Ok(_) => notice::remove(window, cx, "theme-command-save"),
                Err(error) => notice::error(
                    window,
                    cx,
                    "theme-command-save",
                    "Could not save appearance",
                    error.to_string(),
                ),
            });
        })
        .detach();
}

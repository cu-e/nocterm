//! Searchable appearance pickers reached from the workspace command palette.
use std::rc::Rc;

use crate::AppearanceMode;
use crate::notice;
use gpui_kit::{
    App, AppContext as _, Window,
    component::{
        ActiveTheme as _, WindowExt as _,
        command::{Command, CommandItem, CommandState},
    },
    prelude::*,
    px,
};

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
            (name, checked, choice)
        })
        .collect();
    open(
        settings.clone(),
        if dark {
            "Choose dark theme…"
        } else {
            "Choose light theme…"
        },
        options,
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
    .map(|(label, mode)| (label.to_owned(), mode == current, Choice::Scheme(mode)))
    .collect();
    open(
        settings.clone(),
        "Choose color scheme…",
        options,
        window,
        cx,
    );
}

fn open(
    settings: nocterm_store::SettingsHandle,
    placeholder: &'static str,
    options: Vec<(String, bool, Choice)>,
    window: &mut Window,
    cx: &mut App,
) {
    let origin = window.focused(cx);
    let options = Rc::new(options);
    let state = cx.new(|cx| CommandState::new(window, cx));
    let field = state.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let confirm = options.clone();
        let settings = settings.clone();
        let origin = origin.clone();
        dialog.w(px(560.)).p_0().close_button(false).child(
            Command::new(&state)
                .items(
                    options
                        .iter()
                        .map(|(label, checked, _)| {
                            CommandItem::new().label(label.clone()).checked(*checked)
                        })
                        .collect::<Vec<_>>(),
                )
                .placeholder(placeholder)
                .bordered(false)
                .max_h(px(420.))
                .on_confirm(move |index, window, cx| {
                    let Some((_, _, choice)) = confirm.get(index.row) else {
                        return;
                    };
                    let choice = choice.clone();
                    window.close_dialog(cx);
                    if let Some(origin) = &origin {
                        window.focus(origin, cx);
                    }
                    save(choice, &settings, window, cx);
                }),
        )
    });
    field.update(cx, |state, cx| state.focus(window, cx));
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

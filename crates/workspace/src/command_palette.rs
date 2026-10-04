//! The command palette: every command available where focus is, searchable
//! by name and description, with its shortcut.
//!
//! Commands are not listed by hand. The palette asks GPUI which actions the
//! focused view and its ancestors handle (plus application-wide ones), keeps
//! nocterm's own namespaces and runs the chosen one from the focus the
//! palette was opened over, exactly as its shortcut would.
use gpui_kit::{
    Action, App, AppContext as _, FocusHandle, ParentElement as _, SharedString, Styled as _,
    Window,
    component::{
        ActiveTheme as _, WindowExt as _,
        command::{Command, CommandItem, CommandState},
        h_flex,
        kbd::Kbd,
    },
    div,
    prelude::FluentBuilder as _,
    px,
};
use nocterm_keymap::{humanize, offered};
use std::rc::Rc;

struct Entry {
    label: SharedString,
    action: Box<dyn Action>,
    binding: Option<Kbd>,
}

fn entries(window: &Window, cx: &App) -> Vec<Entry> {
    let mut entries: Vec<Entry> = window
        .available_actions(cx)
        .into_iter()
        .filter(|action| offered(action.name()))
        .map(|action| Entry {
            label: humanize(action.name()).into(),
            binding: Kbd::binding_for_action(action.as_ref(), None, window),
            action,
        })
        .collect();
    entries.sort_by(|a, b| a.label.cmp(&b.label));
    entries.dedup_by(|a, b| a.label == b.label);
    entries
}

/// Lets `workspace::ToggleCommandPalette` open the palette in whichever
/// window is active.
pub fn init(cx: &mut App) {
    cx.on_action(|_: &crate::ToggleCommandPalette, cx| {
        if let Some(window) = cx.active_window() {
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| open(window, cx));
            });
        }
    });
}

/// Opens the palette over the focused view.
pub fn open(window: &mut Window, cx: &mut App) {
    let origin = window.focused(cx);
    let entries = Rc::new(entries(window, cx));
    let state = cx.new(|cx| CommandState::new(window, cx));
    let width = px(560.);
    let field = state.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let docs = cx.action_documentation();
        let items = entries.iter().map(|entry| {
            let label = entry.label.clone();
            let binding = entry.binding.clone();
            let description = docs.get(entry.action.name()).copied();
            CommandItem::new()
                .label(label.clone())
                .keywords([SharedString::from(entry.action.name())])
                .keywords(description.map(SharedString::from))
                .child(move |_, cx| {
                    h_flex()
                        .w_full()
                        .gap_3()
                        .child(div().flex_1().min_w_0().truncate().child(label.clone()))
                        .when_some(description, |row, description| {
                            row.child(
                                div()
                                    .max_w(px(220.))
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(description.trim_end_matches('.').to_owned()),
                            )
                        })
                        .children(binding.clone())
                })
        });
        let confirm = entries.clone();
        let origin = origin.clone();
        dialog.w(width).p_0().close_button(false).child(
            Command::new(&state)
                .items(items.collect::<Vec<_>>())
                .placeholder("Run a command…")
                .bordered(false)
                .max_h(px(420.))
                .on_confirm(move |index, window, cx| {
                    let Some(entry) = confirm.get(index.row) else {
                        return;
                    };
                    run(entry.action.boxed_clone(), origin.clone(), window, cx);
                }),
        )
    });
    // The dialog focuses itself as it opens; move focus into the search
    // field after that.
    field.update(cx, |state, cx| state.focus(window, cx));
}

/// Closes the palette and runs `action` as if from `origin`.
fn run(action: Box<dyn Action>, origin: Option<FocusHandle>, window: &mut Window, cx: &mut App) {
    window.close_dialog(cx);
    if let Some(origin) = &origin {
        window.focus(origin, cx);
    }
    window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
}

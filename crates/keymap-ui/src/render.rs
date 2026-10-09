//! The page: a search over a table of commands and their shortcuts.
use gpui_kit::{
    AnyElement, Context, Focusable as _, Keystroke, SharedString, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::Input,
        kbd::Kbd,
        v_flex,
    },
    div,
    prelude::*,
    px, rems,
};
use nocterm_keymap::{Keymap, Source};
use nocterm_ui::{IconName, form};

use crate::{
    rows::{Row, matches, rows},
    view::{KeymapView, Recording},
};

const CONTEXT_WIDTH: f32 = 7.;
const SHORTCUT_WIDTH: f32 = 11.;
const ACTIONS_WIDTH: f32 = 5.5;

fn shortcut(keys: &str) -> AnyElement {
    h_flex()
        .gap_1()
        .children(
            keys.split_whitespace()
                .filter_map(|key| Keystroke::parse(key).ok())
                .map(Kbd::new),
        )
        .into_any_element()
}

impl KeymapView {
    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let searching_keys = self.recording == Some(Recording::Search);
        h_flex()
            .w_full()
            .gap_2()
            .child(
                div().flex_1().child(
                    Input::new(&self.search)
                        .prefix(gpui_kit::component::Icon::new(IconName::Search).small())
                        .cleanable(true),
                ),
            )
            .child(
                Button::new("keymap-search-keys")
                    .ghost()
                    .icon(IconName::Keyboard)
                    .label(if searching_keys {
                        "Press keys…"
                    } else {
                        "Find by keys"
                    })
                    .tooltip("Press a shortcut to see what it runs")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if searching_keys {
                            this.stop_recording(cx);
                        } else {
                            this.record(Recording::Search, cx);
                        }
                    })),
            )
            .into_any_element()
    }

    fn column_titles(cx: &Context<Self>) -> AnyElement {
        h_flex()
            .w_full()
            .px_2()
            .py_1()
            .gap_3()
            .text_xs()
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .text_color(cx.theme().muted_foreground)
            .child(div().flex_1().child("COMMAND"))
            .child(div().w(rems(SHORTCUT_WIDTH)).child("SHORTCUT"))
            .child(div().w(rems(CONTEXT_WIDTH)).child("WHEN"))
            .child(div().w(rems(ACTIONS_WIDTH)))
            .into_any_element()
    }

    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn row(&self, index: usize, row: Row, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, border, accent) = (
            theme.muted_foreground,
            theme.border.opacity(0.5),
            theme.foreground.opacity(0.05),
        );
        let editing = matches!(
            &self.recording,
            Some(Recording::Bind { action, replacing, .. })
                if *action == row.action && *replacing == row.keystrokes
        );
        let keys_cell: AnyElement = if editing {
            div()
                .text_sm()
                .text_color(theme.primary)
                .child("Press keys… (Esc cancels)")
                .into_any_element()
        } else {
            match &row.keystrokes {
                Some(keys) => shortcut(keys),
                None => div()
                    .text_sm()
                    .text_color(muted)
                    .child("—")
                    .into_any_element(),
            }
        };
        let record = Recording::Bind {
            action: row.action.clone(),
            context: row.context.clone(),
            replacing: row.keystrokes.clone(),
        };
        let action = row.action.clone();
        let remove = row
            .keystrokes
            .clone()
            .map(|keys| (row.action.clone(), row.context.clone(), keys));
        let reset = row.action.clone();
        h_flex()
            .id(("keymap-row", index))
            .test_support()
            .w_full()
            .px_2()
            .py_1p5()
            .gap_3()
            .border_t_1()
            .border_color(border)
            .hover(|row| row.bg(accent))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().text_sm().truncate().child(row.label.clone()))
                            .when(row.source == Some(Source::User), |line| {
                                line.child(
                                    div()
                                        .px_1()
                                        .rounded(px(4.))
                                        .bg(accent)
                                        .text_xs()
                                        .text_color(muted)
                                        .child("custom"),
                                )
                            }),
                    )
                    .when(!row.description.is_empty(), |cell| {
                        cell.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .truncate()
                                .child(row.description.clone()),
                        )
                    }),
            )
            .child(
                div()
                    .id(("keymap-keys", index))
                    .test_support()
                    .w(rems(SHORTCUT_WIDTH))
                    .cursor_pointer()
                    .child(keys_cell)
                    .on_click(cx.listener(move |this, _, _, cx| this.record(record.clone(), cx))),
            )
            .child(
                div()
                    .w(rems(CONTEXT_WIDTH))
                    .text_xs()
                    .text_color(muted)
                    .truncate()
                    .child(SharedString::from(row.context.clone().unwrap_or_default())),
            )
            .child(
                h_flex()
                    .w(rems(ACTIONS_WIDTH))
                    .justify_end()
                    .child(
                        Button::new(("keymap-add", index))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .tooltip("Add a shortcut")
                            .on_click(cx.listener(move |this, _, _, cx| this.add(&action, cx))),
                    )
                    .when_some(remove, |cell, (action, context, keys)| {
                        cell.child(
                            Button::new(("keymap-remove", index))
                                .ghost()
                                .xsmall()
                                .icon(IconName::X)
                                .tooltip("Remove this shortcut")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let result =
                                        Keymap::unbind(&action, context.clone(), &keys, cx);
                                    this.report(result, cx);
                                })),
                        )
                    })
                    .when(row.customized, |cell| {
                        cell.child(
                            Button::new(("keymap-reset", index))
                                .ghost()
                                .xsmall()
                                .icon(IconName::RotateCcw)
                                .tooltip("Restore the default shortcuts")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let result = Keymap::reset(&reset, cx);
                                    this.report(result, cx);
                                })),
                        )
                    }),
            )
            .into_any_element()
    }
}

impl Render for KeymapView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_string();
        let listed: Vec<Row> = rows(cx)
            .into_iter()
            .filter(|row| matches(row, &query))
            .collect();
        let count = listed.len();
        let file_error = Keymap::global(cx).error.clone();
        let mut table = v_flex().id("keymap-table").test_support().w_full();
        table = table.child(Self::column_titles(cx));
        for (index, row) in listed.into_iter().enumerate() {
            table = table.child(self.row(index, row, cx));
        }
        if count == 0 {
            table = table.child(form::note("No command matches.", cx));
        }
        v_flex()
            .id("keymap-page")
            .test_support()
            .w_full()
            .gap_3()
            .track_focus(&self.focus_handle(cx))
            .child(form::page_header(
                "Keymap",
                Some(
                    "Every command and its shortcuts. Click a shortcut and press the new keys \
                     to change it. Changes apply at once and are kept in keymap.toml."
                        .into(),
                ),
                cx,
            ))
            .child(self.header(cx))
            .when_some(file_error, |page, error| {
                page.child(form::error_text(
                    format!("keymap.toml could not be used: {error}"),
                    cx,
                ))
            })
            .when_some(self.message.clone(), |page, (message, error)| {
                page.child(if error {
                    form::error_text(message, cx)
                } else {
                    form::note(message, cx)
                })
            })
            .child(table)
    }
}

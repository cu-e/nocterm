//! Literal tool input, separate from the tool's result.
use gpui_kit::{
    App, ClipboardItem, IntoElement, TestSupportExt as _,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_ai::acp;
use nocterm_ui::IconName;
use serde_json::Value;

pub(in crate::panel) struct Source {
    pub label: &'static str,
    pub text: String,
}

pub(in crate::panel) fn source(call: &acp::ToolCall) -> Option<Source> {
    if let Some(input) = call.raw_input.as_ref().filter(|input| !input.is_null()) {
        let literal = ["command", "script", "code"]
            .into_iter()
            .find_map(|key| input.get(key).and_then(Value::as_str));
        return Some(match literal {
            Some(text) => Source {
                label: "Script",
                text: text.to_owned(),
            },
            None => Source {
                label: "Tool input",
                text: input
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| serde_json::to_string_pretty(input).unwrap_or_default()),
            },
        });
    }
    call.title
        .contains(['\n', '\r', '\u{2028}', '\u{2029}'])
        .then(|| Source {
            label: "Tool details",
            text: call.title.clone(),
        })
}

pub(super) fn render(index: usize, source: Source, cx: &App) -> impl IntoElement {
    let text = source.text;
    let copied = text.clone();
    v_flex()
        .id(("tool-input", index))
        .test_support()
        .w_full()
        .min_w_0()
        .max_w_full()
        .gap_1()
        .p_2()
        .rounded(px(6.))
        .bg(cx.theme().muted.opacity(0.5))
        .child(
            h_flex()
                .justify_between()
                .gap_2()
                .text_xs()
                .child(source.label)
                .child(
                    Button::new(("copy-tool-input", index))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Copy)
                        .label("Copy")
                        .accessibility_label("Copy tool input")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
                        }),
                ),
        )
        .child(
            div()
                .id(("tool-input-scroll", index))
                .test_support()
                .w_full()
                .min_w_0()
                .max_w_full()
                .max_h(px(240.))
                .overflow_scroll()
                .flex()
                .items_start()
                .child(
                    div()
                        .id(("tool-input-code", index))
                        .test_support()
                        .flex_shrink_0()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_color(cx.theme().foreground)
                        .text_xs()
                        .whitespace_nowrap()
                        .child(text),
                ),
        )
}

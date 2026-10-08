//! Literal tool input, separate from the tool's result.
use gpui_kit::{
    App, ClipboardItem, IntoElement, SharedString, StyledText, TestSupportExt as _,
    component::{
        ActiveTheme as _, Rope, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        highlighter::SyntaxHighlighter,
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_ai::{
    TerminalCall, acp,
    tool_display::{self, ToolDisplay},
};
use nocterm_ui::IconName;
use serde_json::Value;

pub(in crate::panel) struct Source {
    pub label: &'static str,
    pub text: String,
    pub language: Option<&'static str>,
    pub parameters: Vec<String>,
    pub extra: Vec<Section>,
}

pub(in crate::panel) struct Section {
    pub label: &'static str,
    pub text: String,
    pub language: Option<&'static str>,
}

impl Source {
    fn plain(label: &'static str, text: String, language: Option<&'static str>) -> Self {
        Self {
            label,
            text,
            language,
            parameters: Vec::new(),
            extra: Vec::new(),
        }
    }

    pub(in crate::panel) fn redact(mut self) -> Self {
        let original = self.text.clone();
        self.text = nocterm_ai::redact::redact(&self.text);
        let mut changed = original != self.text;
        self.parameters = self
            .parameters
            .into_iter()
            .map(|text| nocterm_ai::redact::redact(&text))
            .collect();
        for section in &mut self.extra {
            let redacted = nocterm_ai::redact::redact(&section.text);
            changed |= redacted != section.text;
            section.text = redacted;
        }
        if changed && !self.parameters.iter().any(|text| text == "Secrets hidden") {
            self.parameters.push("Secrets hidden".into());
        }
        self
    }
}

pub(in crate::panel) fn source(call: &acp::ToolCall) -> Option<Source> {
    if let Some(display) = ToolDisplay::from_call(call)
        && let Some(request) = display.display_request()
    {
        let mut source = request_source(request);
        if display.redacted {
            source.parameters.push("Secrets hidden".into());
        }
        return Some(source);
    }
    if let Some(request) = tool_display::requested_call(call) {
        let mut source = request_source(request);
        source.label = match source.label {
            "Command" => "Requested command",
            "Input" => "Requested input",
            "Program" => "Requested program",
            "Terminal" => "Requested terminal",
            "Command handle" => "Requested command handle",
            "Connection" => "Requested connection",
            _ => "Requested scope",
        };
        return Some(source);
    }
    if let Some(input) = call.raw_input.as_ref().filter(|input| !input.is_null()) {
        let literal = ["command", "script", "code"]
            .into_iter()
            .find_map(|key| input.get(key).and_then(Value::as_str));
        return Some(match literal {
            Some(text) => Source::plain("Script", text.to_owned(), Some("bash")),
            None => Source::plain(
                "Tool input",
                input
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| serde_json::to_string_pretty(input).unwrap_or_default()),
                (!input.is_string()).then_some("json"),
            ),
        });
    }
    call.title
        .contains(['\n', '\r', '\u{2028}', '\u{2029}'])
        .then(|| Source::plain("Tool details", call.title.clone(), None))
}

fn request_source(call: TerminalCall) -> Source {
    match call {
        TerminalCall::RunCommand(v) => {
            let mut source = Source::plain("Command", v.command, Some("bash"));
            source.parameters = vec![
                format!("Timeout {} ms", v.timeout_ms.unwrap_or(30_000)),
                format!("Idle {} ms", v.idle_ms.unwrap_or(1000)),
            ];
            source
        }
        TerminalCall::SendInput(v) => {
            let mut source = Source::plain("Input", v.text, None);
            source.parameters.push(
                if v.press_enter {
                    "Enter after input"
                } else {
                    "No Enter"
                }
                .into(),
            );
            source
        }
        TerminalCall::ExecCommand(v) => {
            let script = shell_script(&v.program, &v.args);
            let mut source = if let Some(script) = script {
                let mut source = Source::plain("Command", script.to_owned(), Some("bash"));
                source.extra.push(Section {
                    label: "Program",
                    text: v.program,
                    language: None,
                });
                source
            } else {
                Source::plain("Program", v.program, None)
            };
            source.extra.push(Section {
                label: "Arguments",
                text: serde_json::to_string_pretty(&v.args).unwrap_or_default(),
                language: Some("json"),
            });
            if let Some(stdin) = v.stdin {
                source.extra.push(Section {
                    label: "Standard input",
                    text: stdin,
                    language: None,
                });
            }
            source.parameters = vec![
                format!("Timeout {} ms", v.timeout_ms.unwrap_or(30_000)),
                format!("Wait {} ms", v.yield_ms.unwrap_or(1000)),
            ];
            source
        }
        TerminalCall::ReadTerminal(v) => {
            let mut source = Source::plain("Terminal", v.terminal_id, None);
            source
                .parameters
                .push(format!("{} lines", v.lines.unwrap_or(200)));
            if let Some(cursor) = v.since {
                source.parameters.push(format!("Since line {cursor}"));
            }
            source
        }
        TerminalCall::ReadCommand(v) => {
            let mut source = Source::plain("Command handle", v.command_id, None);
            source.parameters = vec![
                format!("Terminal {}", v.terminal_id),
                format!("Wait {} ms", v.yield_ms.unwrap_or(0)),
            ];
            source
        }
        TerminalCall::CancelCommand(v) => {
            let mut source = Source::plain("Command handle", v.command_id, None);
            source
                .parameters
                .push(format!("Terminal {}", v.terminal_id));
            source
        }
        TerminalCall::OpenTerminal(v) => Source::plain("Connection", v.server_id, None),
        TerminalCall::ListTerminals => {
            Source::plain("Scope", "Terminals attached to this chat".into(), None)
        }
    }
}

fn shell_script<'a>(program: &str, args: &'a [String]) -> Option<&'a str> {
    let program = program.rsplit('/').next()?;
    if !matches!(program, "bash" | "sh") || !matches!(args.first()?.as_str(), "-c" | "-lc") {
        return None;
    }
    args.get(1).map(String::as_str)
}

pub(super) fn render(index: usize, source: Source, cx: &App) -> impl IntoElement {
    let mut container = v_flex()
        .id(("tool-input", index))
        .test_support()
        .w_full()
        .min_w_0()
        .max_w_full()
        .gap_1()
        .p_2()
        .rounded(px(6.))
        .bg(cx.theme().muted.opacity(0.5));
    container = container.child(section(
        index,
        None,
        source.label,
        source.text,
        source.language,
        cx,
    ));
    if !source.parameters.is_empty() {
        container = container.child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .children(source.parameters),
        );
    }
    for (part, extra) in source.extra.into_iter().enumerate() {
        container = container.child(section(
            index,
            Some(part),
            extra.label,
            extra.text,
            extra.language,
            cx,
        ));
    }
    container
}

fn section(
    index: usize,
    part: Option<usize>,
    label: &'static str,
    text: String,
    language: Option<&'static str>,
    cx: &App,
) -> impl IntoElement {
    let copy_label = match label {
        "Command" | "Requested command" => "Copy command",
        "Input" | "Requested input" => "Copy input",
        "Program" | "Requested program" => "Copy program",
        "Arguments" => "Copy arguments",
        "Standard input" => "Copy standard input",
        _ => "Copy tool input",
    };
    literal_section(
        index,
        part,
        "tool-input",
        Section {
            label,
            text,
            language,
        },
        copy_label,
        None,
        cx,
    )
}

pub(super) fn literal_section(
    index: usize,
    part: Option<usize>,
    namespace: &'static str,
    section: Section,
    copy_label: &'static str,
    limit: Option<usize>,
    cx: &App,
) -> impl IntoElement {
    let key = |name: String| -> (SharedString, usize) {
        (
            part.map(|part| format!("{name}-extra-{part}"))
                .unwrap_or(name)
                .into(),
            index,
        )
    };
    let scroll_id = key(format!("{namespace}-scroll"));
    let code_id = key(format!("{namespace}-code"));
    let copy_id = key(format!("copy-{namespace}"));
    let Section {
        label,
        text,
        language,
    } = section;
    let copied = text.clone();
    let mut end = limit.unwrap_or(text.len()).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = end < text.len();
    let text = text[..end].to_owned();
    let mut highlighter = language
        .filter(|_| text.len() <= 64 * 1024)
        .map(SyntaxHighlighter::new);
    let highlights = highlighter
        .as_mut()
        .map(|highlighter| {
            highlighter.update(None, &Rope::from(text.as_str()), None);
            highlighter.styles(&(0..text.len()), cx.theme().highlight_theme.as_ref())
        })
        .unwrap_or_default();
    v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .gap_2()
                .text_xs()
                .child(label)
                .child(
                    Button::new(copy_id)
                        .ghost()
                        .xsmall()
                        .icon(IconName::Copy)
                        .label("Copy")
                        .accessibility_label(copy_label)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
                        }),
                ),
        )
        .child(
            div()
                .id(scroll_id)
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
                        .id(code_id)
                        .test_support()
                        .flex_shrink_0()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_color(cx.theme().foreground)
                        .text_xs()
                        .whitespace_nowrap()
                        .child(StyledText::new(text).with_highlights(highlights)),
                ),
        )
        .when(truncated, |section| {
            section.child("Preview limited to 200 KiB. Copy includes the full text.")
        })
}

//! Literal results, independent of ACP request completion and execution identity.
use gpui_kit::{
    App, IntoElement, TestSupportExt as _,
    component::{ActiveTheme as _, h_flex, v_flex},
    prelude::*,
    px,
};
use nocterm_ai::{
    acp,
    tool_display::{self, ToolDisplay, ToolOutcome},
};
use serde_json::Value;

use super::tool_input::{Section, literal_section};
mod decode;
pub(super) use decode::readable;

pub(in crate::panel) struct Output {
    pub sections: Vec<Section>,
    pub parameters: Vec<String>,
}

impl Output {
    fn plain(label: &'static str, text: String) -> Self {
        Self {
            sections: vec![Section {
                label,
                text,
                language: None,
            }],
            parameters: Vec::new(),
        }
    }

    fn append(&mut self, other: Self) {
        self.sections.extend(other.sections);
        self.parameters.extend(other.parameters);
    }

    fn redact(&mut self) {
        let mut changed = false;
        for section in &mut self.sections {
            let redacted = nocterm_ai::redact::redact(&section.text);
            changed |= redacted != section.text;
            section.text = redacted;
        }
        for parameter in &mut self.parameters {
            let redacted = nocterm_ai::redact::redact(parameter);
            changed |= redacted != *parameter;
            *parameter = redacted;
        }
        if changed {
            self.parameters.push("Secrets hidden".into());
        }
    }
}

fn text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| serde_json::to_string_pretty(value).unwrap_or_default())
}

pub(in crate::panel) fn source(call: &acp::ToolCall, redact: bool) -> Option<Output> {
    let display = ToolDisplay::from_call(call);
    if let Some(display) = &display
        && let Some(outcome) = &display.outcome
    {
        let mut output = match outcome {
            ToolOutcome::Pending => return None,
            ToolOutcome::Ok(value) => decode::bridge(&display.tool, value),
            ToolOutcome::Err(error) => Output::plain("Error", error.clone()),
        };
        if redact {
            output.redact();
        }
        return Some(output);
    }
    let tool = display
        .as_ref()
        .map(|display| display.tool.as_str())
        .or_else(|| tool_display::requested_arguments(call).map(|(tool, _)| tool));
    let mut output = Output {
        sections: Vec::new(),
        parameters: Vec::new(),
    };
    let mut content_values = Vec::new();
    for item in &call.content {
        let value = match item {
            acp::ToolCallContent::Content(content) => match &content.content {
                acp::ContentBlock::Text(content) => Value::String(content.text.clone()),
                block => serde_json::to_value(block).unwrap_or_default(),
            },
            item => serde_json::to_value(item).unwrap_or_default(),
        };
        content_values.push(value);
    }
    if let [value] = content_values.as_slice() {
        let decoded = tool.and_then(|tool| decode::known(tool, value, 0));
        output.append(decoded.unwrap_or_else(|| Output::plain("Tool output", text(value))));
    } else if !content_values.is_empty() {
        if let Some(tool) = tool {
            for value in &content_values {
                output.append(
                    decode::known(tool, value, 0)
                        .unwrap_or_else(|| Output::plain("Tool output", text(value))),
                );
            }
        } else {
            output.append(Output::plain(
                "Tool output",
                content_values
                    .iter()
                    .map(text)
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ));
        }
    }
    if let Some(raw) = call.raw_output.as_ref().filter(|raw| !raw.is_null()) {
        // Providers often repeat the same result as both content and rawOutput.
        // Suppress only exact text/JSON equivalence, never an unknown extra block.
        let duplicate = match content_values.as_slice() {
            [] => false,
            [value] => decode::equivalent(tool, value, raw),
            values => decode::equivalent(tool, &Value::Array(values.to_vec()), raw),
        };
        if !duplicate {
            let decoded = tool.and_then(|tool| decode::known(tool, raw, 0));
            output.append(decoded.unwrap_or_else(|| Output::plain("Tool output", text(raw))));
        }
    }
    if output.sections.is_empty() {
        return None;
    }
    if redact {
        output.redact();
    }
    Some(output)
}

pub(super) fn render(index: usize, output: Output, cx: &App) -> impl IntoElement {
    let mut container = v_flex()
        .id(("tool-output", index))
        .test_support()
        .w_full()
        .min_w_0()
        .max_w_full()
        .gap_1()
        .p_2()
        .rounded(px(6.))
        .bg(cx.theme().muted.opacity(0.5));
    if !output.parameters.is_empty() {
        container = container.child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .children(output.parameters),
        );
    }
    let mut remaining = 200 * 1024;
    for (part, section) in output.sections.into_iter().enumerate() {
        let limit = remaining;
        let mut end = section.text.len().min(limit);
        while !section.text.is_char_boundary(end) {
            end -= 1;
        }
        remaining -= end;
        let copy_label = match section.label {
            "Standard output" => "Copy standard output",
            "Standard error" => "Copy standard error",
            "Error" => "Copy error",
            "Terminal output" => "Copy terminal output",
            _ => "Copy tool output",
        };
        container = container.child(literal_section(
            index,
            part.checked_sub(1),
            "tool-output",
            section,
            copy_label,
            Some(limit),
            cx,
        ));
    }
    container
}

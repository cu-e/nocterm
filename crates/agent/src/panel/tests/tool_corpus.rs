//! Rendering regressions use the same detector for fixtures and private saved chats.
use crate::panel::entries::{tool_input, tool_output};
use nocterm_ai::{acp, history::SavedChat, thread::Entry, tool_display};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

#[derive(Default)]
struct RawParts {
    header: bool,
    input: bool,
    output: bool,
}

impl RawParts {
    fn any(&self) -> bool {
        self.header || self.input || self.output
    }
}

fn wrapper_text(text: &str) -> bool {
    if text.contains("<untrusted_tool_result") {
        return true;
    }
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .is_some_and(|object| {
            [
                "result",
                "content",
                "jsonrpc",
                "isError",
                "structuredContent",
            ]
            .iter()
            .any(|key| object.contains_key(*key))
        })
}

fn raw_parts(call: &acp::ToolCall) -> RawParts {
    let header = tool_display::header(call).starts_with("mcp");
    let input = tool_input::source(call).is_some_and(|input| {
        (input.label == "Tool input" && input.language == Some("json"))
            || wrapper_text(&input.text)
            || input
                .extra
                .iter()
                .any(|section| wrapper_text(&section.text))
            || input.parameters.iter().any(|text| wrapper_text(text))
    });
    let output = tool_output::source(call, false).is_some_and(|output| {
        output
            .sections
            .iter()
            .any(|section| wrapper_text(&section.text))
            || output.parameters.iter().any(|text| wrapper_text(text))
    });
    RawParts {
        header,
        input,
        output,
    }
}

fn nocterm_row(call: &acp::ToolCall) -> bool {
    [Some(call.title.as_str()), call.name.as_deref()]
        .into_iter()
        .flatten()
        .any(|name| {
            name.starts_with("mcp.nocterm-")
                || name.starts_with("mcp__nocterm-")
                || name.starts_with("mcp_nocterm_")
        })
        || call
            .raw_input
            .as_ref()
            .and_then(|input| input.get("server"))
            .and_then(Value::as_str)
            .is_some_and(|server| server == "nocterm" || server.starts_with("nocterm-"))
        || tool_display::ToolDisplay::from_call(call).is_some()
}

#[test]
fn provider_tool_row_fixtures_never_show_raw_headers_inputs_or_envelopes() {
    for fixture in [
        include_str!("fixtures/codex-success.json"),
        include_str!("fixtures/codex-error.json"),
        include_str!("fixtures/hermes-fence.json"),
        include_str!("fixtures/hermes-truncated-preview.json"),
        include_str!("fixtures/hermes-legacy-fence.json"),
        include_str!("fixtures/hermes-error.json"),
        include_str!("fixtures/claude-list.json"),
        include_str!("fixtures/claude-string.json"),
        include_str!("fixtures/rejected-request.json"),
    ] {
        let call: acp::ToolCall = serde_json::from_str(fixture).unwrap();
        assert!(nocterm_row(&call));
        let raw = raw_parts(&call);
        assert!(
            !raw.any(),
            "fixture row remained raw: header={} input={} output={}",
            raw.header,
            raw.input,
            raw.output
        );
    }
}

#[derive(Default)]
struct Counts {
    chats: usize,
    rows: usize,
    nocterm: usize,
    raw: usize,
    header: usize,
    input: usize,
    output: usize,
}

fn provider(agent: &str) -> &'static str {
    let agent = agent.to_ascii_lowercase();
    if agent.contains("codex") {
        "Codex"
    } else if agent.contains("claude") {
        "Claude"
    } else if agent.contains("hermes") {
        "Hermes"
    } else {
        "Other"
    }
}

fn measure(directory: &Path) -> Result<BTreeMap<&'static str, Counts>, String> {
    let mut files = std::fs::read_dir(directory)
        .map_err(|_| "Cannot read chat corpus directory")?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|_| "Cannot enumerate chat corpus")
        })
        .collect::<Result<Vec<_>, _>>()?;
    files.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
    });
    files.sort();
    let mut counts = BTreeMap::<_, Counts>::new();
    for (index, path) in files.iter().enumerate() {
        let bytes =
            std::fs::read(path).map_err(|_| format!("Cannot read corpus chat at index {index}"))?;
        let chat: SavedChat = serde_json::from_slice(&bytes)
            .map_err(|_| format!("Invalid JSON/history schema at corpus index {index}"))?;
        let counts = counts.entry(provider(&chat.agent_id)).or_default();
        counts.chats += 1;
        for entry in &chat.entries {
            let Entry::Tool(call) = entry else {
                continue;
            };
            counts.rows += 1;
            if !nocterm_row(call) {
                continue;
            }
            counts.nocterm += 1;
            let raw = raw_parts(call);
            counts.raw += usize::from(raw.any());
            counts.header += usize::from(raw.header);
            counts.input += usize::from(raw.input);
            counts.output += usize::from(raw.output);
        }
    }
    Ok(counts)
}

#[test]
#[ignore = "reads private saved chats only when NOCTERM_CHATS_DIR is set"]
#[expect(
    clippy::print_stdout,
    reason = "explicitly requested aggregate corpus diagnostics"
)]
fn tool_rows_corpus() {
    let directory = std::env::var_os("NOCTERM_CHATS_DIR")
        .expect("set NOCTERM_CHATS_DIR to a saved-chat directory");
    let counts = measure(Path::new(&directory)).unwrap_or_else(|error| panic!("{error}"));
    assert!(!counts.is_empty(), "chat corpus has no JSON histories");
    let mut total = Counts::default();
    for (provider, counts) in counts {
        println!(
            "{provider}: chats={} tool_rows={} nocterm_rows={} raw_rows={} (header={} input={} output={})",
            counts.chats,
            counts.rows,
            counts.nocterm,
            counts.raw,
            counts.header,
            counts.input,
            counts.output
        );
        total.chats += counts.chats;
        total.rows += counts.rows;
        total.nocterm += counts.nocterm;
        total.raw += counts.raw;
    }
    println!(
        "Total: chats={} tool_rows={} nocterm_rows={} raw_rows={}",
        total.chats, total.rows, total.nocterm, total.raw
    );
}

#[test]
fn corpus_measurement_reports_malformed_history_without_private_contents() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("private-name.json"),
        "private invalid contents",
    )
    .unwrap();
    let error = measure(directory.path()).err().unwrap();
    assert!(error.contains("Invalid JSON/history schema"));
    assert!(!error.contains("private"));
}

#[test]
fn detector_counts_a_row_once_even_when_all_three_parts_are_raw() {
    let call = acp::ToolCall::new("raw", "mcp_nocterm_exec_command")
        .raw_input(serde_json::json!({"server":"nocterm-99","tool":"exec_command","arguments":{}}))
        .raw_output(serde_json::json!({"result":"x","content":[]}));
    let raw = raw_parts(&call);
    assert!(raw.header && raw.input && raw.output);
    assert_eq!(usize::from(raw.any()), 1);
}

//! Readable conversation text, preserving complete messages and tool results.
use super::Entry;
use crate::model::{
    content::{Content, ResourceBody},
    tools::ToolContent,
};
use std::fmt::Write as _;

pub fn conversation_text(title: &str, entries: &[Entry]) -> String {
    let mut text = format!("# {title}\n\n");
    for entry in entries {
        match entry {
            Entry::User(parts) => {
                text.push_str("## User\n\n");
                for part in parts {
                    content(&mut text, part);
                }
            }
            Entry::Agent(value) => {
                let _ = writeln!(text, "## Assistant\n\n{value}");
            }
            Entry::Thought(value) => {
                let _ = writeln!(text, "## Reasoning\n\n{value}");
            }
            Entry::Content(value) => {
                text.push_str("## Attachment\n\n");
                content(&mut text, value);
            }
            Entry::Tool(tool) => {
                let _ = writeln!(
                    text,
                    "## Tool: {}\n\nStatus: {:?}\n",
                    tool.title, tool.status
                );
                if let Some(input) = &tool.raw_input {
                    let _ = writeln!(text, "Input:\n{input:#}\n");
                }
                for value in &tool.content {
                    match value {
                        ToolContent::Content(value) => content(&mut text, &value.content),
                        ToolContent::Diff(diff) => {
                            let _ = writeln!(
                                text,
                                "File: {}\nBefore:\n{}\nAfter:\n{}",
                                diff.path.display(),
                                diff.old_text.as_deref().unwrap_or_default(),
                                diff.new_text
                            );
                        }
                        ToolContent::Terminal(terminal) => {
                            let _ = writeln!(text, "Terminal: {}", terminal.terminal_id);
                        }
                    }
                }
                if let Some(output) = &tool.raw_output {
                    let _ = writeln!(text, "Output:\n{output:#}");
                }
            }
        }
        text.push('\n');
    }
    text
}

fn content(text: &mut String, content: &Content) {
    match content {
        Content::Text(value) => {
            let _ = writeln!(text, "{}", value.text);
        }
        Content::Image(value) => {
            let _ = writeln!(
                text,
                "[Image: {}]",
                value.uri.as_deref().unwrap_or(&value.mime_type)
            );
        }
        Content::Audio(value) => {
            let _ = writeln!(text, "[Audio: {}]", value.mime_type);
        }
        Content::ResourceReference(value) => {
            let _ = writeln!(text, "[Resource: {}] {}", value.name, value.uri);
        }
        Content::EmbeddedResource(value) => match &value.resource {
            ResourceBody::Text(value) => {
                let _ = writeln!(text, "{}\n{}", value.uri, value.text);
            }
            ResourceBody::Blob(value) => {
                let _ = writeln!(text, "[Resource: {}]", value.uri);
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tools::ToolRecord;

    #[test]
    fn conversation_export_keeps_full_text_reasoning_and_tool_output_in_order() {
        let long = format!("{}\ncomplete ending", "answer ".repeat(50_000));
        let entries = vec![
            Entry::User(vec![Content::text("first\nsecond")]),
            Entry::Thought("check the server".into()),
            Entry::Tool(
                ToolRecord::new("tool-1", "Read logs")
                    .raw_input(Some(serde_json::json!({"path":"server.log"})))
                    .content(vec![
                        Content::text("complete tool text").into(),
                        ToolContent::Diff(crate::model::tools::FileDiff {
                            path: "config.toml".into(),
                            old_text: Some("old settings".into()),
                            new_text: "new settings".into(),
                            meta: None,
                        }),
                    ])
                    .raw_output(Some(serde_json::json!({"stdout":"all log lines"}))),
            ),
            Entry::Agent(long.clone()),
        ];
        let text = conversation_text("Audit", &entries);
        assert!(text.starts_with("# Audit\n\n## User\n\nfirst\nsecond\n"));
        assert!(text.contains("## Reasoning\n\ncheck the server"));
        assert!(text.contains("## Tool: Read logs"));
        assert!(text.contains("all log lines"));
        assert!(text.contains("server.log"));
        assert!(text.contains("complete tool text"));
        assert!(text.contains("File: config.toml\nBefore:\nold settings\nAfter:\nnew settings"));
        assert!(text.ends_with(&format!("## Assistant\n\n{long}\n\n")));
        assert!(text.find("## Reasoning") < text.find("## Tool"));
    }
}

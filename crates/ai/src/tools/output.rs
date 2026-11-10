//! Agent-facing observations are display data, never execution capabilities.
use nocterm_session::command_jobs::CommandSnapshot;
use serde::{Deserialize, Serialize};

/// A bounded terminal snapshot; overlapping line numbers replace earlier text.
#[derive(Serialize, Deserialize)]
pub struct TerminalReport {
    pub text: String,
    pub first_line: u64,
    pub next_line: u64,
    pub cursor_semantics: String,
    pub truncated: bool,
    pub alt_screen: bool,
}
impl TerminalReport {
    pub const CURSOR_SEMANTICS: &str = "inclusive snapshot; replace overlapping lines";

    pub fn new(
        text: String,
        first_line: u64,
        next_line: u64,
        truncated: bool,
        alt_screen: bool,
    ) -> Self {
        Self {
            text,
            first_line,
            next_line,
            cursor_semantics: Self::CURSOR_SEMANTICS.into(),
            truncated,
            alt_screen,
        }
    }

    /// An observation timeout or idle output does not establish process completion.
    pub fn observed(self, completion: String) -> ObservationReport {
        ObservationReport {
            completed: completion == "prompt_returned",
            terminal: self,
            completion,
            exit_status: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct ObservationReport {
    #[serde(flatten)]
    pub terminal: TerminalReport,
    pub completion: String,
    pub completed: bool,
    pub exit_status: Option<u32>,
}

/// A snapshot of an admitted command. Its identifier carries no control authority.
#[derive(Serialize, Deserialize)]
pub struct CommandReport {
    pub command_id: String,
    pub state: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_status: Option<u32>,
    pub total_bytes: u64,
    pub truncated: bool,
    pub error: Option<String>,
}
impl CommandReport {
    pub fn new(id: &str, snapshot: &CommandSnapshot, redact: bool) -> Self {
        let text = |value: String| {
            if redact {
                crate::redact::redact(&value)
            } else {
                value
            }
        };
        Self {
            command_id: id.into(),
            state: snapshot.state().as_str().into(),
            stdout: text(String::from_utf8_lossy(snapshot.stdout()).into_owned()),
            stderr: text(
                snapshot
                    .exit()
                    .map(|exit| exit.stderr.clone())
                    .unwrap_or_default(),
            ),
            exit_status: snapshot.exit().and_then(|exit| exit.status),
            total_bytes: snapshot.total_bytes(),
            truncated: snapshot.total_bytes() > snapshot.stdout().len() as u64,
            error: snapshot
                .error()
                .map(|error| crate::redact::redact(&error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests;

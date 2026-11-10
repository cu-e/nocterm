//! File transfer tools: arguments, bounds and the agent-facing transfer report.
//!
//! Absolute-path rules are endpoint specific and belong to the transfer request
//! (`nocterm_transfers::Request::between`); this module only bounds the shape.
use super::TerminalCallError;
use nocterm_transfers::{Progress, TransferDirection, TransferState};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAX_TRANSFER_SOURCES: usize = 64;
pub const MAX_TRANSFER_PATH_BYTES: usize = 4096;
pub const MAX_TRANSFER_SOURCE_BYTES: usize = 64 * 1024;
pub const MAX_TRANSFER_FAILURES: usize = 16;
const MAX_ID_BYTES: usize = 128;
const MAX_YIELD_MS: u64 = 10_000;

/// Copy files and folders between the hosts of two attached terminals.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransferFiles {
    /// terminal_id whose host holds the sources; a local terminal means this computer.
    pub source_terminal_id: String,
    /// Absolute paths of files or folders on the source host; folders copy recursively.
    pub sources: Vec<String>,
    /// terminal_id whose host receives the copies.
    pub destination_terminal_id: String,
    /// Absolute path of an existing folder on the destination host.
    pub destination: String,
    /// Replace files that already exist at the destination; by default they are skipped.
    #[serde(default)]
    pub overwrite: bool,
    pub yield_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadTransfer {
    pub transfer_id: String,
    pub yield_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelTransfer {
    pub transfer_id: String,
}

fn id_len_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_BYTES
}

/// Control characters (NUL, newlines, escapes) could reshape the approval card text.
fn path_ok(path: &str) -> bool {
    !path.is_empty() && path.len() <= MAX_TRANSFER_PATH_BYTES && !path.chars().any(char::is_control)
}

impl TransferFiles {
    pub(super) fn validate(&self) -> Result<(), TerminalCallError> {
        if !id_len_ok(&self.source_terminal_id) || !id_len_ok(&self.destination_terminal_id) {
            return Err(TerminalCallError::InvalidTarget);
        }
        let total: usize = self.sources.iter().map(String::len).sum();
        if self.sources.is_empty()
            || self.sources.len() > MAX_TRANSFER_SOURCES
            || !self.sources.iter().all(|path| path_ok(path))
            || !path_ok(&self.destination)
            || total > MAX_TRANSFER_SOURCE_BYTES
            || self.yield_ms.is_some_and(|n| n > MAX_YIELD_MS)
        {
            return Err(TerminalCallError::TransferRejected);
        }
        Ok(())
    }
}

impl ReadTransfer {
    pub(super) fn validate(&self) -> Result<(), TerminalCallError> {
        if id_len_ok(&self.transfer_id) && self.yield_ms.is_none_or(|n| n <= MAX_YIELD_MS) {
            Ok(())
        } else {
            Err(TerminalCallError::ReadTransferRejected)
        }
    }
}

impl CancelTransfer {
    pub(super) fn validate(&self) -> Result<(), TerminalCallError> {
        if id_len_ok(&self.transfer_id) {
            Ok(())
        } else {
            Err(TerminalCallError::InvalidTransferId)
        }
    }
}

fn state_name(state: TransferState) -> &'static str {
    match state {
        TransferState::Queued => "queued",
        TransferState::Discovering => "discovering",
        TransferState::Running => "running",
        TransferState::Completed => "completed",
        TransferState::Failed => "failed",
        TransferState::Cancelled => "cancelled",
    }
}

// Seam: `TransferDirection::Relay` arrives with the transfers domain contract; the
// fallthrough already names it, so this function needs no change when it lands.
fn direction_name(direction: TransferDirection) -> &'static str {
    if direction == TransferDirection::Upload {
        "upload"
    } else if direction == TransferDirection::Download {
        "download"
    } else {
        "relay"
    }
}

/// Agent-facing JSON for one transfer; paths and error text are redacted on request.
pub fn report(progress: &Progress, redact: bool) -> Value {
    let text = |text: &str| {
        if redact {
            crate::redact::redact(text)
        } else {
            text.to_owned()
        }
    };
    let failures: Vec<Value> = progress
        .errors
        .iter()
        .take(MAX_TRANSFER_FAILURES)
        .map(|issue| {
            json!({
                "path": issue.path().map(|path| text(&path.to_string())),
                "operation": issue.operation().to_string(),
                "error": text(&issue.cause().to_string()),
            })
        })
        .collect();
    json!({
        "transfer_id": progress.id.to_string(),
        "state": state_name(progress.state),
        "direction": direction_name(progress.direction),
        "destination": text(&progress.destination),
        "files": {
            "discovered": progress.discovered_files,
            "completed": progress.completed_files,
            "skipped": progress.skipped_files,
            "failed": progress.failed_files,
        },
        "bytes": {"total": progress.total_bytes, "transferred": progress.sent_bytes},
        "discovery_complete": progress.discovery_complete,
        "failures": failures,
        "failures_truncated": progress.errors.len() > MAX_TRANSFER_FAILURES,
    })
}

#[cfg(test)]
mod tests;

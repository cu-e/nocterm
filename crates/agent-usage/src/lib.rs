//! Bounded native reads of provider account plan limits.
//!
//! Owns: Claude's account usage request and its response grammar, the read of
//! the OAuth token Claude Code keeps in its configuration directory, Codex's
//! session-log tail reader and grammar, and where both live on this machine
//! (`native_sources`).
//! Does not own: which agent reports limits which way (the agent's
//! `UsageProfile`), scheduling, merging or presenting limits (agent runtime
//! and feature), or token refresh: an expired token yields no report until
//! Claude Code renews it.
//! Ports provided: AI `AccountUsageSource` per `AccountUsage` kind. Ports used:
//! none.
//! Settings section: none. Globals: none.
//! Invariants: readers serve only the agents `UsageSources` routes to them by
//! profile; the token is sent only to Anthropic's API over TLS and never
//! logged; requests are bounded by a timeout, log reads by a tail size, and
//! both run when the runtime polls on its background executor.
mod claude;
mod parser;
mod session_log;

pub use claude::ClaudeUsageReader;
pub use session_log::CodexUsageReader;

use nocterm_ai::{agents::AccountUsage, usage::UsageSources};
use std::{path::PathBuf, sync::Arc};

/// The account sources for this machine, with their locations resolved once
/// from `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and `HOME`.
pub fn native_sources() -> UsageSources {
    UsageSources::default()
        .with(
            AccountUsage::OAuthAccount,
            Arc::new(ClaudeUsageReader::new(home_dir(
                "CLAUDE_CONFIG_DIR",
                ".claude",
            ))),
        )
        .with(
            AccountUsage::SessionLog,
            Arc::new(CodexUsageReader::new(home_dir("CODEX_HOME", ".codex"))),
        )
}

/// `variable` when set, otherwise `default` under the home directory.
fn home_dir(variable: &str, default: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(default)))
}

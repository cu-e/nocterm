//! Bounded native reads of provider account plan limits over the network.
//!
//! Owns: Claude's account usage request and its response grammar, and the
//! read of the OAuth token Claude Code keeps in its configuration directory.
//! Does not own: scheduling, merging or presenting limits (agent runtime and
//! feature), Codex's session-log reader (`nocterm-agent-storage`), or token
//! refresh: an expired token yields no report until Claude Code renews it.
//! Ports provided: AI `AccountUsageSource`. Ports used: none.
//! Settings section: none. Globals: none.
//! Invariants: only Claude launches read the token; it is sent only to
//! Anthropic's API over TLS and never logged; requests are bounded by a
//! timeout and run when the runtime polls on its background executor.
mod claude;
mod parser;

pub use claude::ClaudeUsageReader;

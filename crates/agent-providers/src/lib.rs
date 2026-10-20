//! The ACP agents nocterm ships, and how each deviates from plain ACP.
//!
//! Owns: one module per built-in agent returning its `AgentProfile` (pinned
//! launch, home state, plan-limit reporting, brand mark) and `catalog()`.
//! Does not own: dialect and resolution rules (`nocterm-ai`), launching
//! (`nocterm-acp`), native usage reads (`nocterm-agent-usage`) or rendering.
//! Ports used: none. Ports provided: none; the application injects `catalog()`.
//! Settings section: none. Globals: none.
//! Invariants: ids are unique and valid; launches are pinned; every profile
//! passes the conformance tests without editing them.
mod claude;
mod codex;
mod hermes;
#[cfg(test)]
mod tests;

use nocterm_ai::agents::{AgentCatalog, AgentProfile};

/// The built-in agents, in display order.
pub const PROFILES: &[AgentProfile] = &[claude::PROFILE, codex::PROFILE, hermes::PROFILE];

/// The catalog the application injects into agent services.
pub fn catalog() -> AgentCatalog {
    AgentCatalog::new(PROFILES.iter().copied()).expect("built-in agent profiles are valid")
}

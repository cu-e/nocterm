//! Agent contracts, bounded context rules and legacy version-one history formats.
//!
//! Owns: configuration, session/tool rules, bounded history codecs and storage ports.
//! Does not own: history/favorites files or persistence scheduling; repositories are adapters.
//! Ports used: none for history codecs. Ports provided: AgentConnector, ToolBridge,
//! AgentStateRepository, ChatRepository and usage::AccountUsageSource.
//! Settings section: ai. Globals: none.
//! Invariants: saved records validate identity/version and encoded/structural limits.
//! Existing ACP-backed DTOs, sandbox/image I/O and clock/entropy are explicit
//! migration debt; storage ports add no new protocol-specific types.
pub use agent_client_protocol_schema::v1 as acp;
pub mod approval;
pub mod commands;
pub mod connection;
pub mod context;
pub mod env;
pub mod favorites;
pub mod history;
pub mod images;
pub mod mcp;
pub mod redact;
pub mod registry;
pub mod sandbox;
pub mod session;
pub mod session_config;
mod settings;
#[cfg(feature = "test-support")]
pub mod testing;
pub mod thread;
pub mod time;
pub mod tool_display;
pub mod tools;
pub mod usage;
pub use connection::*;
pub use registry::{AgentLaunch, AgentRegistry};
pub use settings::lifecycle::{AgentResourceSettings, AgentSessionSettings, InvalidResourceLimits};
pub use settings::{
    AgentServerSettings, AiSettings, ApprovalPolicy, ApprovalSettings, SandboxMode,
};
pub use tools::*;
#[cfg(test)]
mod tests;

pub mod storage;

pub mod model;

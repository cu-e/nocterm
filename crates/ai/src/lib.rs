//! Agent contracts, neutral protocol records, bounded context rules and scoped history formats.
//!
//! Owns: neutral `model` records (the only agent vocabulary other crates see), agent
//! profiles, catalog and settings resolution (`agents`), configuration rules, transcript reduction, session/tool rules, pure sandbox policy,
//! bounded history codecs, immutable scoped execution origins and capability/storage ports.
//! Does not own: per-chat configuration/composer state, native workspace/sandbox lookup,
//! history/favorites files or persistence scheduling; native capabilities and repositories are adapters.
//! Ports used: none for history codecs. Ports provided: metadata::ChatMetadataSource,
//! AgentConnector, ToolBridge,
//! AgentStateRepository, ChatRepository, usage::AccountUsageSource,
//! workspace::WorkdirResolver, sandbox::SandboxProbe and
//! images::{ImageInspector, ImageThumbnailer}.
//! Image preflight and encoded admission are pure; native decoding is injected.
//! ImageCodecCause reuses core shared causes without exposing a native codec type.
//! Account usage diagnostics retain typed native I/O causes through foundation SharedError;
//! narrow domain constructors preserve operation category and path.
//! Workspace preparation and sandbox spawn diagnostics retain typed shared native I/O;
//! adapter constructors preserve classification without changing policy or mount authority.
//! Settings section: ai. Globals: none.
//! Invariants: saved records validate identity/version and encoded/structural limits.
//! Snapshot framing and transcript timestamps accept supplied identity/time.
//! Native chat allocation and wall-clock facts are provided by ChatMetadataSource.
//! No agent protocol SDK type appears in this crate; `nocterm-acp` maps the SDK to neutral records.
//! Legacy transcripts preserve their version-one bytes. Newly framed documents use version
//! two with checked explicit context ownership; saved ownership never grants execution access.
//! Agent launch settings validation is pure; native text parsing and page controls belong to Agent.
//! No concrete agent is named here: built-in profiles are injected as an `AgentCatalog`.
pub mod agents;
pub mod approval;
pub mod commands;
pub mod connection;
pub mod context;
pub mod env;
pub mod execution_origin;
pub mod favorites;
pub mod history;
pub mod images;
pub mod mcp;
pub mod metadata;
pub mod redact;
pub mod sandbox;
pub mod session;
pub mod session_config;
mod settings;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod thread;
pub mod tool_display;
pub mod tools;
pub mod usage;
pub mod workspace;
pub use agents::{AgentCatalog, AgentLaunch, AvailableAgents};
pub use connection::*;
pub use settings::lifecycle::{AgentResourceSettings, AgentSessionSettings, InvalidResourceLimits};
pub use settings::{
    AgentServerSettings, AgentValidationError, AiSettings, ApprovalPolicy, ApprovalSettings,
    SandboxMode, validate_agent,
};
pub use tools::*;
#[cfg(test)]
mod tests;

pub mod storage;

pub mod model;

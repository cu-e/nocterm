//! Runtime-neutral agent contracts and bounded, explicitly selected terminal context.
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
pub use settings::lifecycle::{AgentResourceSettings, AgentSessionSettings};
pub use settings::{
    AgentServerSettings, AiSettings, ApprovalPolicy, ApprovalSettings, SandboxMode,
};
pub use tools::*;
#[cfg(test)]
mod tests;

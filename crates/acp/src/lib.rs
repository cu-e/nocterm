//! ACP subprocess transport and authenticated local terminal-tool relay.
//!
//! Owns: ACP subprocess drivers, process cleanup and the authenticated local relay.
//! Does not own: chat presentation, history storage or terminal permission decisions.
//! Ports used: AI connector and tool-bridge contracts; provided: `AgentConnector`, `ToolBridge`
//! and, on Linux, the `run_agent_host` entry point with its `AgentHostError`.
//! Settings section: none. Globals: none.
//! Invariants: relay registrations authenticate callers; revocation closes owned sockets.
mod bridge;
mod client;
mod lines;
mod models;
mod process;
mod relay;

pub use bridge::{BridgeServer, RelayCommand};
pub use client::AcpConnector;
#[cfg(target_os = "linux")]
pub use process::{AgentHostError, run_agent_host};
pub use relay::{run_relay, run_relay_from_environment};

#[cfg(test)]
mod mapping;

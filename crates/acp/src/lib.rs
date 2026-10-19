//! ACP subprocess transport and authenticated local terminal-tool relay.
//!
//! Owns: the only mapping between the agent protocol SDK and `nocterm_ai::model`, ACP subprocess drivers, process cleanup, native workspace preparation,
//! sandbox launch/probing and the authenticated local relay.
//! Does not own: chat presentation, history storage or terminal permission decisions.
//! Ports used: AI connector, tool-bridge, workspace and sandbox contracts; provided:
//! `AgentConnector`, `ToolBridge`, `NativeWorkdirResolver`, `NativeSandboxProbe`
//! and, on Linux, the `run_agent_host` entry point with its `AgentHostError`.
//! Settings section: none. Globals: none.
//! Invariants: SDK types never leave this crate; relay registrations authenticate callers; revocation closes owned sockets.
//! Native taskkill diagnostics retain the original I/O error through shared typed causes;
//! the public process shutdown port retains its existing string diagnostic.
mod bridge;
mod client;
mod lines;
mod mapping;
mod models;
mod process;
mod relay;
mod sandbox;
mod workspace;
pub use sandbox::NativeSandboxProbe;
pub use workspace::NativeWorkdirResolver;

pub use bridge::{BridgeServer, RelayCommand};
pub use client::AcpConnector;
#[cfg(target_os = "linux")]
pub use process::{AgentHostError, run_agent_host};
pub use relay::{run_relay, run_relay_from_environment};

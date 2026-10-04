//! ACP subprocess transport and authenticated local terminal-tool relay.
mod bridge;
mod client;
mod lines;
mod models;
mod process;
mod relay;

pub use bridge::BridgeServer;
pub use client::AcpConnector;
pub use relay::run_relay;

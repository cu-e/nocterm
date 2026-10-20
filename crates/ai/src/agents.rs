//! Which agents nocterm knows and which ones the user can start.
//!
//! An [`AgentProfile`] describes how a known agent deviates from plain ACP: its
//! pinned launch, the home directories it writes, how it reports plan limits and
//! its brand mark. Deviations are named by shape, never by brand, so an agent
//! whose shapes already exist needs only a profile. The application supplies the
//! profiles as an [`AgentCatalog`]; this crate names no concrete agent.
//! [`AvailableAgents`] resolves the catalog and the user's settings into the
//! agents that can be started.
mod available;
mod catalog;
mod launch;
mod profile;
#[cfg(test)]
mod tests;

pub use available::{AgentConfigWarning, AgentSource, AvailableAgent, AvailableAgents};
pub use catalog::{AgentCatalog, CatalogError};
pub use launch::{AgentLaunch, valid_id};
pub use profile::{AccountUsage, AgentMark, AgentProfile, LaunchDefaults, UsageMeta, UsageProfile};

//! Native host-driven Rig engine for explicitly configured Cloud providers.
//!
//! Owns: provider HTTP I/O, bounded progress, SDK stepping and derived checkpoints.
//! Does not own: grants, tools, SSH, durable effects, journals, secrets discovery,
//! account sign-in, pricing, subscription credentials, GPUI or conversation views.
//! Ports provided: cloud-model AgentEngineFactory/AgentEngine. Ports used:
//! cloud-model EngineEventSink and ModelCallAdmission; the host persists progress and gates every effect.
//! Settings sections: none; configuration is injected. Globals: none.
//! Invariants: SDK tools never execute; only a finished provider turn exposes a
//! tool batch. Checkpoints carry no provider key and are not an execution ledger.
//! Every provider HTTP send waits for the host's explicit durable reservation;
//! a per-engine once scope checks cancellation, provider and current host authority.
//! Provider credentials remain in this adapter's explicit memory-only transport.

mod attempt;
mod call;
mod checkpoint;
mod config;
mod driver;
mod failure;
mod http;
mod lease;
mod limits;
mod mapping;
mod progress;
mod state;

pub use config::{ProviderBinding, ProviderEndpoint, ProviderKind, RigEngineFactory};
pub use lease::{MAX_PROVIDER_LEASE, ProviderLease};

#[cfg(test)]
mod tests;

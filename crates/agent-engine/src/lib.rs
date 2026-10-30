//! Native host-driven Rig engine for explicitly configured Cloud providers.
//!
//! Owns: provider HTTP I/O, bounded progress, SDK stepping and derived checkpoints.
//! Does not own: grants, tools, SSH, durable effects, journals, secrets discovery,
//! account sign-in, pricing, subscription credentials, GPUI or conversation views.
//! Ports provided: cloud-model AgentEngineFactory/AgentEngine and CoordinatorProviders.
//! Ports used: EngineEventSink, ModelCallAdmission and ProviderCredentialSource;
//! the host persists progress and gates every effect. The provider catalog binds
//! trusted endpoint/model/pricing policy to original finite memory-only grants.
//! Settings sections: none; configuration is injected. Globals: none.
//! Invariants: SDK tools never execute; only a finished provider turn exposes a
//! tool batch. Checkpoints carry no provider key and are not an execution ledger.
//! Every provider HTTP send waits for the host's explicit durable reservation;
//! a per-engine once scope checks cancellation, provider and current host authority.
//! Provider credentials remain in this adapter's explicit memory-only transport.
//! A provider lease is capped at [`MAX_PROVIDER_LEASE`] per renewal step, not per chat:
//! `LeaseServices` lets the host renew it in the background while `DevicePresence`
//! reports an owner device online (pure `RenewalPolicy`, the clock is the host's tokio
//! timer) and report accepted call usage through `ModelUsageObserver`, counts only.
//! `ProviderEndpoint::broker` addresses the pool host's credential broker, where the
//! lease's "key" is a virtual token and the broker substitutes the real API key.

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
mod provider_catalog;
mod state;

pub use config::{ProviderBinding, ProviderEndpoint, ProviderKind, RigEngineFactory};
pub use lease::{
    DevicePresence, LeaseRenewal, LeaseServices, MAX_PROVIDER_LEASE, ModelUsageObserver,
    ProviderLease, RenewalError, RenewalPolicy, RenewalPolicyError, RenewalSetup,
};
pub use provider_catalog::{ConfiguredProviderCatalog, ProviderPolicy};

#[cfg(test)]
mod tests;

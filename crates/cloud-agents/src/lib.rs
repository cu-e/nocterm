//! The Cloud agents of the known hubs, shared by every window.
//!
//! Owns: one catalog per tracked hub (the agents the account may see, kept fresh by the hub's
//! change hints and retried with growing delays), and the order of a start: reserving a seat,
//! waiting for the person to trust an unknown pool, registering the runner, ready. Also the
//! writes of a person's own agents through the same entity, stopping or removing the person's
//! seat of an agent, and the web panel address the hub states for each tracked hub. While a hub
//! is tracked, its bound Cloud chats are swept every minute so their runners' SSH certificate
//! requests are answered inside each chat's binding (`ssh_renewal`, ADR 0147).
//!
//! Does not own: the hub protocol, sealing, signatures or pins (the injected ports are
//! implemented by `nocterm-hub-io`), the rules that group agents (`nocterm-cloud-agents-model`),
//! views, and opening the chat of a started agent (the caller does, from [`CloudAgentsEvent`]).
//!
//! Ports used: [`CloudAgentCatalog`], [`CloudAgentLauncher`], `AgentSeats`, `ChatSshCertifier`
//! and [`PersonalAgents`] from
//! `nocterm-cloud-agents-model`, passed in [`Deps`]. Provides: [`Installed`] with the
//! [`CloudAgents`] entity.
//!
//! Settings: none. Globals: none.
//!
//! Invariants: a hub that answers `UNIMPLEMENTED` is reported [`CatalogStatus::Unsupported`]
//! and is not polled again; a start never skips the trust step of an unpinned pool and is
//! idempotent, so repeating it after the person trusted the pool continues the same seat; an
//! unpinned pool is never trusted without an explicit [`CloudAgents::trust_pool`]; a dropped
//! hub forgets its agents and stops its watcher; a stopped or removed seat forgets its start,
//! so the next start resumes or provisions a seat.

mod catalogs;
mod installed;
mod service;
mod ssh_renewal;
mod starts;
#[cfg(test)]
mod tests;

pub use catalogs::CatalogStatus;
pub use installed::{Deps, Installed, install};
pub use service::{CloudAgents, CloudAgentsEvent};
pub use starts::{StartId, StartPhase, StartView};

//! The device side of Cloud chats, run on the contract's sans-I/O device core
//! (`nocterm-cloud-device`, ADR 0144).
//!
//! The core decides everything a device does: hub trust, verification, the transcript, the
//! commands and the order of the steps. This module only performs the effects a flow asks for
//! ([`effects`] over the hub client's Cloud calls and the hub's Store and agent services; the
//! device key stays inside `nocterm-hub-client`), keeps the core's sealed
//! checkpoints in private files ([`checkpoints`]), maps the core's display values to the
//! neutral conversation values ([`mapping`]), keeps the sealed commands on their way so they
//! resume after a restart ([`outbox`]) and implements the `CloudConversation` port over the `OpenChat` flow
//! ([`conversation`]).
//! Subscriber delivery has finite message and shared byte budgets; native file work has
//! bounded admission and retains its permits through physical completion after cancellation.
mod account;
mod checkpoints;
mod conversation;
mod deliveries;
pub(crate) mod driver;
mod effects;
mod mapping;
mod outbox;
mod prompt;
mod providers;
mod requests;
mod server_flows;
mod servers;
mod storage_work;
mod subscriptions;
mod update_size;

pub(crate) use account::{DeviceAccount, DeviceAccountError};
pub(crate) use checkpoints::{AccountPlace, Checkpoints, directory_file, remove_earlier_pages};
pub(crate) use conversation::{ChatSetup, DeviceConversation};
pub(crate) use outbox::Outbox;
pub(crate) use providers::{ProviderDirectory, ProviderModels, lease_models};
pub(crate) use servers::ServerDirectory;
#[cfg(test)]
pub(crate) use servers::chosen as chosen_servers;
pub(crate) use storage_work::StorageWork;
pub(crate) use subscriptions::UpdateBudget;

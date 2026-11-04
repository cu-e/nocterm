//! The device side of Cloud chats, run on the contract's sans-I/O device core
//! (`nocterm-cloud-device`, ADR 0136).
//!
//! The core decides everything a device does: hub trust, verification, the transcript, the
//! commands and the order of the steps. This module only performs the effects a flow asks for
//! ([`effects`] over [`rpc`] and the hub's Store and agent services), keeps the core's sealed
//! checkpoints in private files ([`checkpoints`]), maps the core's display values to the
//! neutral conversation values ([`mapping`]), keeps the exact publications of commands on
//! their way ([`outbox`]) and implements the `CloudConversation` port over the `OpenChat` flow
//! ([`conversation`]).
mod account;
mod checkpoints;
mod conversation;
pub(crate) mod driver;
mod effects;
mod mapping;
mod outbox;
mod rpc;

pub(crate) use account::{DeviceAccount, DeviceAccountError};
pub(crate) use checkpoints::{AccountPlace, Checkpoints, directory_file, remove_earlier_pages};
pub(crate) use conversation::{ChatSetup, DeviceConversation};
pub(crate) use outbox::Outbox;

//! Controlled native protocol framing and neutral materialization.
//!
//! Source buffers and scanner workspace use the supplied original operation
//! budget before allocation. JSON-RPC payloads remain source spans through SDK
//! routing; neutral materializers consume only actual prepaid native values.
//! This module does not install a fallback to the generic typed SDK decoder.
mod authentication;
mod budget;
mod channel;
mod close_command;
mod commands;
mod configuration_command;
mod configuration_copy;
mod content;
mod controls;
mod copy;
mod diagnostics;
mod driver;
mod error;
mod events;
mod fields;
mod handler;
mod initialize;
mod legacy_models;
mod lists;
mod literal;
mod members;
mod models;
mod notice;
mod outgoing;
pub(crate) mod permission;
mod permission_task;
mod redaction;
mod results;
mod rpc;
mod scan;
mod state;
mod stderr;
mod tools;
mod transport;
mod usage;
mod value;
mod wire;

#[cfg(test)]
mod tests;

pub(crate) use budget::SourceBudget;

mod arguments;
mod diagnostic_text;
mod environment;
mod environment_snapshot;
mod native_process;

mod initialization_command;
mod native_admission;
mod native_connect;
mod native_driver;
mod native_launch;
mod native_session;
pub(crate) use native_connect::Preparation as ConnectionPreparation;
pub use redaction::BuildError as DiagnosticBuildError;
pub(crate) use redaction::DiagnosticRedactor;

mod permission_schema;

//! Explicit trusted session targets and actual retained physical login ownership.
#[cfg(target_os = "linux")]
mod broker;
pub(crate) mod config;
#[cfg(target_os = "linux")]
mod login;
#[cfg(target_os = "linux")]
mod publication;
#[cfg(target_os = "linux")]
mod resources;
#[cfg(target_os = "linux")]
mod setup;
#[cfg(target_os = "linux")]
mod source;
#[cfg(target_os = "linux")]
mod targets;
#[cfg(target_os = "linux")]
mod task;
#[cfg(target_os = "linux")]
pub(crate) use broker::NativeSessionBroker;
#[cfg(target_os = "linux")]
pub(crate) use resources::{SessionCapacity, SessionResources};
#[cfg(target_os = "linux")]
pub(crate) use setup::SessionBrokerSetup;

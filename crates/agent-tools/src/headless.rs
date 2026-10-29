//! Native run-scoped terminal tools with explicit authority and durable job boundaries.
mod commands;
mod dispatch;
mod error;
mod executor;
mod initial;
mod live;
mod pending_open;
mod physical;
mod presentation;
mod projection;
mod registry;
mod retained;
mod runtime;
mod session_watch;
mod terminal_failure;
mod view;
pub use error::HeadlessError;
pub use executor::{HeadlessToolServices, HeadlessTools};
pub use initial::{
    InitialSessionCandidate, SessionGeneration, SessionLifecycleEvent, SessionObservation,
    SessionObservations,
};
pub use registry::{
    HeadlessSessionInfo, HeadlessSessionStatus, HeadlessSessionTask, HeadlessTerminalOptions,
    SessionRegistry,
};
pub use retained::RetainedTerminals;
pub use runtime::RetainedToolsFactory;

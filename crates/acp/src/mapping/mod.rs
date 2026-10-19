//! The only place agent-protocol SDK records and neutral `nocterm-ai` records meet.
mod content;
mod controls;
mod info;
mod requests;
mod session;
mod tools;

pub(crate) use controls::options;
pub(crate) use info::{auth_methods, capabilities, remote_error};
pub(crate) use requests::{
    new_session, opened, permission, prompt, prompt_outcome, servers, set_config, set_mode,
    wire_outcome, wire_session,
};
pub(crate) use session::notice;

#[cfg(test)]
mod equivalence_tests;
#[cfg(test)]
mod session_tests;

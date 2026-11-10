//! Refusals at command registry admission and lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum JobError {
    #[error("This chat already has four active commands. Read or cancel one first.")]
    Busy,
    #[error("Unknown command for this chat and terminal.")]
    Unknown,
}

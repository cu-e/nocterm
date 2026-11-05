//! One change of a transcript, as the implementation of the port hands it out.
use crate::{Approval, PendingCommand, Row, Run, ServerBinding};

/// What changed since the last delta. [`crate::CloudTranscript::apply`] applies it in this
/// order: on `reset` everything is cleared; rows whose index is below `first_index` are dropped;
/// when `changed_from` is set the rows from that index on are dropped and `rows` are appended;
/// runs, approvals, pending commands and servers that are present replace the held ones; the scalar
/// fields are taken as they are.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConversationDelta {
    pub reset: bool,
    pub first_index: u64,
    pub changed_from: Option<u64>,
    pub rows: Vec<Row>,
    pub runs: Option<Vec<Run>>,
    pub approvals: Option<Vec<Approval>>,
    pub pending: Option<Vec<PendingCommand>>,
    /// The chat's current binding (ADR 0139).
    pub servers: Option<ServerBinding>,
    pub control_revision: u64,
    pub deleted: bool,
    pub omitted_earlier: bool,
}

/// The agent's task list for the current turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub entries: Vec<PlanEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanEntry {
    pub content: String,
    pub priority: PlanPriority,
    pub status: PlanStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanPriority {
    High,
    Medium,
    Low,
}

/// Variant names are shown to the user through `Debug`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

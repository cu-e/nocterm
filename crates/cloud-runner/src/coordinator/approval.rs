//! Who answers for a mutating tool call. By default a person does; an embedding
//! runner may relax that for targets it contains itself (ADR 0127).
use nocterm_ai::TerminalCall;
use nocterm_cloud_model::authority::TargetBinding;

/// Decides whether a mutating call on `target` waits for a person's approval.
/// Read-only calls never ask. Without a policy every mutating call asks.
pub trait ToolApprovalPolicy: Send + Sync {
    fn requires_approval(&self, call: &TerminalCall, target: &TargetBinding) -> bool;
}

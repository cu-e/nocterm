//! `approval: "never"` of the managed policy (ADR 0127): a mutating call asks
//! nobody when it runs in this runner's own containment, and still asks a person
//! for every target the runner does not contain.
use nocterm_ai::TerminalCall;
use nocterm_cloud_model::authority::TargetBinding;
use nocterm_cloud_runner::coordinator::ToolApprovalPolicy;

pub(crate) struct ContainedApproval {
    contained: Vec<TargetBinding>,
}
impl ContainedApproval {
    pub(crate) fn new(contained: Vec<TargetBinding>) -> Self {
        Self { contained }
    }
}
impl ToolApprovalPolicy for ContainedApproval {
    fn requires_approval(&self, _call: &TerminalCall, target: &TargetBinding) -> bool {
        !self.contained.contains(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_ai::ExecCommand;
    use nocterm_cloud_model::identity::{Fingerprint, NamespaceId};

    fn target(byte: u8) -> TargetBinding {
        TargetBinding::new(
            Fingerprint::from_bytes([byte; 32]),
            Fingerprint::from_bytes([byte; 32]),
            NamespaceId::new("agent".to_owned()).unwrap(),
            Fingerprint::from_bytes([byte; 32]),
        )
    }
    fn exec() -> TerminalCall {
        TerminalCall::ExecCommand(ExecCommand {
            terminal_id: "t".into(),
            program: "/bin/true".into(),
            args: Vec::new(),
            stdin: None,
            timeout_ms: None,
            yield_ms: None,
        })
    }

    #[test]
    fn only_a_contained_target_is_trusted_and_every_other_still_asks() {
        let policy = ContainedApproval::new(vec![target(1)]);
        assert!(!policy.requires_approval(&exec(), &target(1)));
        assert!(policy.requires_approval(&exec(), &target(2)));
        assert!(ContainedApproval::new(Vec::new()).requires_approval(&exec(), &target(1)));
    }
}

//! A runner policy decides which mutating calls wait for a person (ADR 0127).
use super::*;
use nocterm_ai::TerminalCall;
use nocterm_cloud_model::{authority::TargetBinding, payload::BoundedJson};
use reconciliation::{admitted, tool_turn};

struct Contained(bool);
impl ToolApprovalPolicy for Contained {
    fn requires_approval(&self, _call: &TerminalCall, _target: &TargetBinding) -> bool {
        !self.0
    }
}

async fn run_one_tool(fixture: &Fixture) -> RunBinding {
    fixture.engine.push(tool_turn());
    fixture.engine.push(complete());
    fixture
        .tools
        .push(Ok(BoundedJson::new(serde_json::json!({"exit":0})).unwrap()));
    admitted(fixture).await
}

#[tokio::test]
async fn a_policy_that_trusts_the_target_runs_a_mutating_call_without_a_person() {
    let fixture = Fixture::with_approval_policy(Arc::new(Contained(true)));
    let run = run_one_tool(&fixture).await;
    fixture.actor.drive_one().await.unwrap();
    assert_eq!(fixture.tools.requests().len(), 1);
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::Completed
    );
    assert!(
        fixture
            .events
            .events()
            .iter()
            .all(|event| !matches!(event, CoordinatorEvent::Approval { .. }))
    );
}

#[tokio::test]
async fn a_policy_that_does_not_trust_the_target_still_waits_for_a_person() {
    let fixture = Fixture::with_approval_policy(Arc::new(Contained(false)));
    let run = run_one_tool(&fixture).await;
    let actor = fixture.actor.clone();
    let worker = tokio::spawn(async move { actor.drive_one().await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while fixture.actor.status(&run).await.unwrap() != RunStatus::WaitingApproval {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(fixture.tools.requests().is_empty());
    worker.abort();
}

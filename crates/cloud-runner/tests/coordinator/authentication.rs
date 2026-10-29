//! A retained terminal refusal preserves native authentication classification.
use super::reconciliation::{admitted, tool_turn};
use super::*;

#[tokio::test]
async fn a_nested_native_unlock_failure_stays_waiting_and_dispatches_no_tool() {
    let fixture = Fixture::new();
    fixture.engine.push(tool_turn());
    fixture.tools.require_unlock_on_next_attach();
    let run = admitted(&fixture).await;
    let error = fixture.actor.drive_one().await.unwrap_err();
    assert!(matches!(error, CoordinatorError::Tool(ref cause) if cause.is_needs_unlock()));
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::NeedsUnlock
    );
    assert!(fixture.tools.requests().is_empty());
    assert!(!fixture.actor.drive_one().await.unwrap());
    let page = fixture
        .journal
        .read(stream(), JournalSequence::default(), limits())
        .await
        .unwrap();
    assert!(!page.records().iter().any(|stored| matches!(
        stored.record().kind(),
        JournalRecordKind::Intent(_) | JournalRecordKind::Dispatched { .. }
    )));
    assert_eq!(fixture.engine.calls(), 1);
}

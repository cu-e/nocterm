//! Retention keeps real control state and never returns DTO capacity first.
use super::*;
use crate::cloud::events::snapshot::DecodedProjection;
use nocterm_core::allocation::AllocationLease;
use nocterm_hub_model::cloud::events::CloudRunStatus;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct PayloadDrop(Arc<AtomicBool>);
impl Drop for PayloadDrop {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
struct AllocationReturn {
    _actual: AllocationLease,
    live: Arc<AtomicBool>,
    violations: Arc<AtomicUsize>,
}
impl Drop for AllocationReturn {
    fn drop(&mut self) {
        if self.live.load(Ordering::SeqCst) {
            self.violations.fetch_add(1, Ordering::SeqCst);
        }
    }
}
#[test]
fn runner_snapshots_drop_actual_dto_before_returning_its_permit_on_prepare_or_consume_failure() {
    for prepare_succeeds in [false, true] {
        let work = Fixture::budget(1024);
        let live = Arc::new(AtomicBool::new(true));
        let violations = Arc::new(AtomicUsize::new(0));
        let actual = work
            .try_reserve(PublicationAllocation::new(64, 1).unwrap())
            .unwrap();
        let allocation = AllocationLease::new(
            64,
            1,
            AllocationReturn {
                _actual: actual,
                live: live.clone(),
                violations: violations.clone(),
            },
        );
        let owner = DecodedProjection::new(PayloadDrop(live.clone()), allocation);
        let result: Result<(), JournalError> = owner.finish(
            |_| {
                if prepare_succeeds {
                    Ok(())
                } else {
                    Err(JournalError::Capacity)
                }
            },
            |_payload| Err(JournalError::Capacity),
        );
        assert!(matches!(result, Err(JournalError::Capacity)));
        assert!(!live.load(Ordering::SeqCst));
        assert_eq!(violations.load(Ordering::SeqCst), 0);
        assert!(
            work.try_reserve(PublicationAllocation::new(1024, 1).unwrap())
                .is_ok()
        );
    }
    let fixture = Fixture::new();
    let (next, input, time) = two_events(&fixture);
    let work = Fixture::budget(256 * 1024 * 1024);
    assert!(matches!(
        SnapshotProjection::new(&fixture.material, &work).encode(
            &fixture.lease,
            next,
            &input,
            SnapshotLimits::new(4096, 16 * 1024 * 1024, 1).unwrap(),
            time
        ),
        Err(JournalError::Capacity)
    ));
    assert!(
        work.try_reserve(PublicationAllocation::new(256 * 1024 * 1024, 1).unwrap())
            .is_ok()
    );
}
#[test]
fn runner_snapshots_keep_pending_control_and_unknown_job_roots_under_pressure_then_preserve_deleted()
 {
    let fixture = Fixture::new();
    let mut history = History::new(&fixture);
    history.prompt(&fixture.run, 1);
    let old = history.tool(&fixture.run, 21);
    history.background_result(&old);
    history.status(&fixture.run, RunStatus::Running);
    let current = newer_run(&fixture);
    history.prompt(&current, 2);
    history.status(&fixture.run, RunStatus::Completed);
    history.status(&current, RunStatus::Running);
    history.usage(&current, Some(99));
    history.usage(&current, None);
    let pending = history.tool(&current, 31);
    history.approval(&pending);
    history.status(&current, RunStatus::WaitingApproval);
    history.text(
        &current,
        &"x".repeat(nocterm_cloud_model::payload::MAX_PROGRESS_BYTES),
    );
    history.background(&old, BackgroundOutcome::Unknown);
    let work = Fixture::budget(256 * 1024 * 1024);
    let projection = SnapshotProjection::new(&fixture.material, &work);
    let tight = SnapshotLimits::new(4096, 16 * 1024 * 1024, 16 * 1024).unwrap();
    let snapshot = projection
        .encode(
            &fixture.lease,
            history.next_snapshot(),
            &history.source(None),
            tight,
            history.time(),
        )
        .unwrap();
    check_snapshot(&fixture, &snapshot, |body| {
        assert_pending_roots(body, &fixture, &current, &old, &pending)
    });
    history.continue_after(&snapshot);
    history.delete();
    history.background(&old, BackgroundOutcome::ObservedExit(7));
    let input = history.source(Some(snapshot));
    let deletion = projection
        .encode(
            &fixture.lease,
            history.next_snapshot(),
            &input,
            tight,
            history.time(),
        )
        .unwrap();
    check_snapshot(&fixture, &deletion, |body| {
        assert!(
            body.events
                .iter()
                .any(|e| matches!(e.payload, CloudEventPayload::Deleted { .. }))
        );
        assert!(body.events.iter().any(|e|matches!(&e.payload,CloudEventPayload::PromptAccepted {run,..} if run.id==*fixture.run.run().as_bytes())));
        assert!(body.events.iter().any(|e|matches!(&e.payload,CloudEventPayload::ToolIntent(intent) if intent.id==*old.intent.id().as_bytes())));
        let Some(last) = body.events.last() else {
            panic!("missing terminal fact")
        };
        assert!(
            matches!(&last.payload,CloudEventPayload::BackgroundLifecycle(fact) if fact.outcome==BackgroundOutcome::ObservedExit(7) && fact.job.execution.run.id==*fixture.run.run().as_bytes())
        );
        assert!(body.events.iter().any(|e|matches!(&e.payload,CloudEventPayload::BackgroundLifecycle(fact) if fact.outcome==BackgroundOutcome::Unknown)));
    });
    assert!(matches!(
        projection.encode(
            &fixture.lease,
            history.next_snapshot(),
            &input,
            SnapshotLimits::new(2, 16 * 1024 * 1024, 16 * 1024).unwrap(),
            history.time()
        ),
        Err(JournalError::Capacity)
    ));
}
fn assert_pending_roots(
    body: &CloudSnapshotPayload<'_>,
    fixture: &Fixture,
    current: &RunBinding,
    old: &history::OriginalTool,
    pending: &history::OriginalTool,
) {
    assert!(body.omitted_earlier);
    assert!(
        !body
            .events
            .iter()
            .any(|e| matches!(e.payload, CloudEventPayload::Text { .. }))
    );
    for run in [&fixture.run, current] {
        assert!(body.events.iter().any(|e|matches!(&e.payload,CloudEventPayload::PromptAccepted {run:seen,..} if seen.id==*run.run().as_bytes())));
    }
    for tool in [old, pending] {
        assert!(body.events.iter().any(|e|matches!(&e.payload,CloudEventPayload::ToolIntent(intent) if intent.id==*tool.intent.id().as_bytes())));
    }
    assert!(body.events.iter().any(|e|matches!(&e.payload,CloudEventPayload::Approval {intent,..} if intent.id==*pending.intent.id().as_bytes())));
    assert!(body.events.iter().any(|e|matches!(&e.payload,CloudEventPayload::ToolResult {observation,..} if observation.intent==*old.intent.id().as_bytes() && matches!(observation.effect,nocterm_hub_model::cloud::events::ObservedEffect::Background(_)))));
    let status = body.events.iter().rev().find_map(|e| match e.payload {
        CloudEventPayload::Status { run, status } if run.id == *current.run().as_bytes() => {
            Some(status)
        }
        _ => None,
    });
    assert_eq!(status, Some(CloudRunStatus::WaitingApproval));
    assert!(body.events.iter().any(|event| matches!(event.payload,
        CloudEventPayload::Status {run,status:CloudRunStatus::Running} if run.id==*current.run().as_bytes())));
    let usage = body
        .events
        .iter()
        .rev()
        .find_map(|e| match e.payload {
            CloudEventPayload::Usage { run, usage } if run.id == *current.run().as_bytes() => {
                Some(usage)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(usage.cost_micros, None);
    assert_eq!(usage.input_tokens, Some(7));
}
#[test]
fn runner_snapshots_prune_completed_unreferenced_tool_history_as_a_closed_group() {
    let fixture = Fixture::new();
    let mut history = History::new(&fixture);
    history.prompt(&fixture.run, 1);
    let old = history.tool(&fixture.run, 21);
    history.background_result(&old);
    history.background(&old, BackgroundOutcome::ObservedExit(0));
    history.status(&fixture.run, RunStatus::Completed);
    let current = newer_run(&fixture);
    history.prompt(&current, 2);
    history.status(&current, RunStatus::Completed);
    let work = Fixture::budget(256 * 1024 * 1024);
    let projection = SnapshotProjection::new(&fixture.material, &work);
    let baseline = projection
        .encode(
            &fixture.lease,
            history.next_snapshot(),
            &history.source(None),
            limits(),
            history.time(),
        )
        .unwrap();
    history.continue_after(&baseline);
    let input = history.source(Some(baseline));
    let selected = projection
        .encode(
            &fixture.lease,
            history.next_snapshot(),
            &input,
            SnapshotLimits::new(2, 16 * 1024 * 1024, 8 * 1024 * 1024).unwrap(),
            history.time(),
        )
        .unwrap();
    check_snapshot(&fixture, &selected, |body| {
        assert!(body.omitted_earlier);
        assert_eq!(body.events.len(), 2);
        assert!(
            matches!(body.events[0].payload,CloudEventPayload::PromptAccepted {run,..} if run.id==*current.run().as_bytes())
        );
        assert!(
            matches!(body.events[1].payload,CloudEventPayload::Status {run,status:CloudRunStatus::Completed} if run.id==*current.run().as_bytes())
        );
    });
}

#[test]
fn runner_snapshots_preserve_selected_terminal_run_until_the_queued_run_actually_starts() {
    let fixture = Fixture::new();
    let mut history = History::new(&fixture);
    history.prompt(&fixture.run, 1);
    history.status(&fixture.run, RunStatus::Running);
    let queued = newer_run(&fixture);
    history.prompt(&queued, 2);
    history.status(&fixture.run, RunStatus::Completed);
    let work = Fixture::budget(256 * 1024 * 1024);
    let projection = SnapshotProjection::new(&fixture.material, &work);
    let baseline = projection
        .encode(
            &fixture.lease,
            history.next_snapshot(),
            &history.source(None),
            limits(),
            history.time(),
        )
        .unwrap();
    history.continue_after(&baseline);
    let input = history.source(Some(baseline));
    assert!(matches!(
        projection.encode(
            &fixture.lease,
            history.next_snapshot(),
            &input,
            SnapshotLimits::new(2, 16 * 1024 * 1024, 8 * 1024 * 1024).unwrap(),
            history.time()
        ),
        Err(JournalError::Capacity)
    ));
    let selected = projection
        .encode(
            &fixture.lease,
            history.next_snapshot(),
            &input,
            SnapshotLimits::new(3, 16 * 1024 * 1024, 8 * 1024 * 1024).unwrap(),
            history.time(),
        )
        .unwrap();
    check_snapshot(&fixture, &selected, |body| {
        assert!(body.omitted_earlier);
        assert_eq!(body.events.len(), 3);
        assert!(
            matches!(body.events[0].payload,CloudEventPayload::PromptAccepted {run,..} if run.id==*fixture.run.run().as_bytes())
        );
        assert!(
            matches!(body.events[1].payload,CloudEventPayload::PromptAccepted {run,..} if run.id==*queued.run().as_bytes())
        );
        assert!(
            matches!(body.events[2].payload,CloudEventPayload::Status {run,status:CloudRunStatus::Completed} if run.id==*fixture.run.run().as_bytes())
        );
    });
}

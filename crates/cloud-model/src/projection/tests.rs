use super::{testing::*, *};
use crate::{
    authority::RunBinding,
    coordinator::{CoordinatorEvent, RunStatus, ToolObservation},
    engine::{EngineEvent, EngineText, TextChannel, UsageObservation},
    identity::*,
    payload::BoundedJson,
    run::ToolIntent,
    tool::ToolEffectStatus,
};
use nocterm_ai::{
    TerminalCall, conversation::ConversationPhase, model::tools::ToolStatus, thread::Entry,
    tool_display::ToolDisplay,
};
use serde_json::json;

fn prompt(run: &RunBinding, id: u8) -> CoordinatorEvent {
    prompt_event(run, "question", CommandId::from_bytes([id; 16]))
}
fn text(run: &RunBinding, channel: TextChannel, value: &str) -> CoordinatorEvent {
    CoordinatorEvent::Progress {
        run: run.clone(),
        event: EngineEvent::Text(EngineText::new(channel, value.into()).unwrap()),
    }
}
fn status(run: &RunBinding, status: RunStatus) -> CoordinatorEvent {
    CoordinatorEvent::Status {
        run: run.clone(),
        status,
    }
}
fn state(
    projection: &ConversationProjection,
) -> (
    ProjectionCursor,
    ProjectionCursor,
    ChatControlRevision,
    ConversationPhase,
    serde_json::Value,
    Option<RunBinding>,
    usize,
    u64,
) {
    (
        projection.cursor(),
        projection.known_head(),
        projection.control(),
        projection.phase(),
        serde_json::to_value(&projection.transcript().entries).unwrap(),
        projection.run().cloned(),
        projection.resident_bytes(),
        projection.updated_at(),
    )
}
fn tool(run: &RunBinding, id: u8) -> (ToolIntent, TerminalCall, ToolDisplay) {
    let call = TerminalCall::ListTerminals;
    let intent = ToolIntent::new(
        IntentId::from_bytes([id; 16]),
        StepId::from_bytes([id; 16]),
        execution_authority(run.clone()),
        Fingerprint::from_bytes([id; 32]),
        &call,
    )
    .unwrap();
    let display = ToolDisplay::new(&call, Some("original target".into()));
    (intent, call, display)
}

#[test]
fn verified_duplicate_pages_keep_one_prompt_and_render_existing_semantic_rows() {
    let mut fixture = ProjectionFixture::new();
    let run = run_binding(1);
    let mut projection = ConversationProjection::new(fixture.binding().clone());
    let first = fixture.page(vec![prompt(&run, 1)]);
    assert_eq!(projection.apply(&first).unwrap().len(), 1);
    assert!(projection.apply(&first).unwrap().is_empty());
    let page = fixture.page(vec![
        status(&run, RunStatus::Running),
        text(&run, TextChannel::Thought, "thinking"),
        text(&run, TextChannel::Assistant, "answer"),
        CoordinatorEvent::Progress {
            run: run.clone(),
            event: EngineEvent::Usage(UsageObservation {
                input_tokens: Some(3),
                output_tokens: Some(4),
                total_tokens: Some(7),
                ..UsageObservation::default()
            }),
        },
    ]);
    projection.apply(&page).unwrap();
    assert_eq!(projection.transcript().entries.len(), 3);
    assert!(matches!(
        &projection.transcript().entries[0],
        Entry::User(_)
    ));
    assert!(
        matches!(&projection.transcript().entries[1], Entry::Thought(value) if value=="thinking")
    );
    assert!(matches!(&projection.transcript().entries[2], Entry::Agent(value) if value=="answer"));
    assert_eq!(projection.transcript().tokens.as_ref().unwrap().total, 7);
    assert_eq!(projection.phase(), ConversationPhase::Running);
}

#[test]
fn wrong_source_or_writer_in_a_page_leaves_every_published_value_unchanged() {
    for writer in [false, true] {
        let mut fixture = ProjectionFixture::new();
        let run = run_binding(1);
        let mut projection = ConversationProjection::new(fixture.binding().clone());
        projection
            .apply(&fixture.page(vec![prompt(&run, 1)]))
            .unwrap();
        let before = state(&projection);
        let page = fixture.page(vec![
            status(&run, RunStatus::Running),
            text(&run, TextChannel::Assistant, "must roll back"),
        ]);
        let mut events = page.events().to_vec();
        let foreign = if writer {
            ProjectionBinding::new(run.stream(), WriterEpoch::new(2).unwrap())
        } else {
            ProjectionBinding::new(
                RunBinding::new(
                    run.scope().clone(),
                    run.runner(),
                    run.runner_epoch(),
                    CloudChatId::from_bytes([99; 16]),
                    run.run(),
                    run.epoch(),
                )
                .stream(),
                WriterEpoch::new(1).unwrap(),
            )
        };
        events[1].binding = foreign;
        let page =
            ProjectionBatch::new(fixture.binding().clone(), events, page.known_head()).unwrap();
        assert_eq!(
            projection.apply(&page).unwrap_err(),
            ProjectionError::WrongSource
        );
        assert_eq!(state(&projection), before);
    }
}

#[test]
fn gaps_conflicting_duplicates_and_head_rollbacks_are_transactional_refusals() {
    let mut fixture = ProjectionFixture::new();
    let run = run_binding(1);
    let mut projection = ConversationProjection::new(fixture.binding().clone());
    let first = fixture.page(vec![prompt(&run, 1)]);
    projection.apply(&first).unwrap();
    let before = state(&projection);
    let missing = fixture.page(vec![status(&run, RunStatus::Running)]);
    let gap = fixture.page(vec![text(&run, TextChannel::Assistant, "gap")]);
    assert_eq!(
        projection.apply(&gap).unwrap_err(),
        ProjectionError::Discontinuous
    );
    assert_eq!(state(&projection), before);
    let mut duplicate = first.events().to_vec();
    duplicate[0].head = Fingerprint::from_bytes([99; 32]);
    let conflict =
        ProjectionBatch::new(fixture.binding().clone(), duplicate, first.known_head()).unwrap();
    assert_eq!(
        projection.apply(&conflict).unwrap_err(),
        ProjectionError::Discontinuous
    );
    assert_eq!(state(&projection), before);
    let rollback = ProjectionBatch::new(
        fixture.binding().clone(),
        Vec::new(),
        ProjectionCursor::origin(),
    )
    .unwrap();
    assert_eq!(
        projection.apply(&rollback).unwrap_err(),
        ProjectionError::Rollback
    );
    assert_eq!(state(&projection), before);
    projection.apply(&missing).unwrap();
    projection.apply(&gap).unwrap();
}

#[test]
fn control_revision_rollback_cannot_partially_publish_a_page() {
    let mut fixture = ProjectionFixture::new();
    let run = run_binding(1);
    let mut projection = ConversationProjection::new(fixture.binding().clone());
    fixture.set_control(ChatControlRevision::from_u64(9));
    projection
        .apply(&fixture.page(vec![prompt(&run, 1)]))
        .unwrap();
    let before = state(&projection);
    fixture.set_control(ChatControlRevision::from_u64(8));
    assert_eq!(
        projection
            .apply(&fixture.page(vec![text(&run, TextChannel::Assistant, "old control")]))
            .unwrap_err(),
        ProjectionError::Rollback
    );
    assert_eq!(state(&projection), before);
}

#[test]
fn queued_prompt_does_not_retarget_the_running_conversation() {
    let mut fixture = ProjectionFixture::new();
    let old = run_binding(1);
    let next = run_binding(2);
    let mut projection = ConversationProjection::new(fixture.binding().clone());
    projection
        .apply(&fixture.page(vec![
            prompt(&old, 1),
            status(&old, RunStatus::Running),
            prompt(&next, 2),
            status(&next, RunStatus::Queued),
            text(&old, TextChannel::Assistant, "old still running"),
        ]))
        .unwrap();
    assert_eq!(projection.run(), Some(&old));
    assert_eq!(projection.phase(), ConversationPhase::Running);
    projection
        .apply(&fixture.page(vec![
            status(&old, RunStatus::Completed),
            status(&next, RunStatus::Running),
            text(&next, TextChannel::Assistant, "next"),
        ]))
        .unwrap();
    assert_eq!(projection.run(), Some(&next));
    let before = state(&projection);
    assert_eq!(
        projection
            .apply(&fixture.page(vec![text(
                &old,
                TextChannel::Assistant,
                "late old progress"
            )]))
            .unwrap_err(),
        ProjectionError::WrongRun
    );
    assert_eq!(state(&projection), before);
}

#[test]
fn historical_tool_results_keep_original_rows_when_intent_ids_repeat_in_new_runs() {
    let mut fixture = ProjectionFixture::new();
    let old = run_binding(1);
    let next = run_binding(2);
    let (old_intent, call, display) = tool(&old, 7);
    let (next_intent, _, _) = tool(&next, 7);
    let mut projection = ConversationProjection::new(fixture.binding().clone());
    projection
        .apply(&fixture.page(vec![
            prompt(&old, 1),
            CoordinatorEvent::ToolIntent {
                intent: old_intent.clone(),
                call: call.clone(),
                display: display.clone(),
            },
            status(&old, RunStatus::Completed),
            prompt(&next, 2),
            status(&next, RunStatus::Running),
            CoordinatorEvent::ToolIntent {
                intent: next_intent,
                call,
                display: display.clone(),
            },
        ]))
        .unwrap();
    let observation = ToolObservation::failed(
        old_intent.id(),
        BoundedJson::new(json!({"error":"unknown original effect"})).unwrap(),
        display,
        ToolEffectStatus::Unknown,
    );
    projection
        .apply(&fixture.page(vec![CoordinatorEvent::ToolResult {
            run: old,
            observation,
        }]))
        .unwrap();
    let rows = projection
        .transcript()
        .entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Tool(record) => Some(record),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].id, rows[1].id);
    assert_eq!(rows[0].status, ToolStatus::Failed);
    assert_eq!(rows[1].status, ToolStatus::Running);
    assert_eq!(
        rows[0].raw_output.as_ref().unwrap()["error"],
        "unknown original effect"
    );
    assert_eq!(projection.run(), Some(&next));
}

#[test]
fn decoded_replay_page_bound_counts_payload_bytes_and_json_structure() {
    let fixture = ProjectionFixture::new();
    let run = run_binding(1);
    let oversized = prompt_event(
        &run,
        &"x".repeat(MAX_REPLAY_BYTES / 2),
        CommandId::from_bytes([1; 16]),
    );
    let event = ProjectionEvent {
        binding: fixture.binding().clone(),
        id: EventId::from_bytes([1; 16]),
        before: ProjectionCursor::origin(),
        head: Fingerprint::from_bytes([1; 32]),
        control: ChatControlRevision::default(),
        time: Timestamp::from_millis(1),
        event: oversized,
    };
    assert!(matches!(
        ProjectionBatch::new(
            fixture.binding().clone(),
            vec![event],
            ProjectionCursor::at(EventSequence::from_u64(1), Fingerprint::from_bytes([1; 32]))
        ),
        Err(ProjectionError::Capacity)
    ));
    let (intent, _, display) = tool(&run, 1);
    let observation = ToolObservation::failed(
        intent.id(),
        BoundedJson::new(json!(vec![false; 17000])).unwrap(),
        display,
        ToolEffectStatus::Unknown,
    );
    let event = ProjectionEvent {
        binding: fixture.binding().clone(),
        id: EventId::from_bytes([2; 16]),
        before: ProjectionCursor::origin(),
        head: Fingerprint::from_bytes([2; 32]),
        control: ChatControlRevision::default(),
        time: Timestamp::from_millis(1),
        event: CoordinatorEvent::ToolResult { run, observation },
    };
    assert!(matches!(
        ProjectionBatch::new(
            fixture.binding().clone(),
            vec![event],
            ProjectionCursor::at(EventSequence::from_u64(1), Fingerprint::from_bytes([2; 32]))
        ),
        Err(ProjectionError::Capacity)
    ));
}

#[test]
fn deletion_rejects_new_events_without_changing_published_history() {
    let mut fixture = ProjectionFixture::new();
    let run = run_binding(1);
    let mut projection = ConversationProjection::new(fixture.binding().clone());
    projection
        .apply(&fixture.page(vec![
            prompt(&run, 1),
            CoordinatorEvent::Deleted {
                command: CommandId::from_bytes([2; 16]),
            },
        ]))
        .unwrap();
    let before = state(&projection);
    assert_eq!(
        projection
            .apply(&fixture.page(vec![text(&run, TextChannel::Assistant, "after delete")]))
            .unwrap_err(),
        ProjectionError::InvalidRecord
    );
    assert_eq!(state(&projection), before);
}

#[test]
fn row_capacity_failure_keeps_the_previous_full_transcript_and_cursor() {
    let mut fixture = ProjectionFixture::new();
    let base = run_binding(1);
    let mut projection = ConversationProjection::new(fixture.binding().clone());
    for start in (0..MAX_PROJECTION_ENTRIES).step_by(MAX_REPLAY_EVENTS) {
        let events = (start..(start + MAX_REPLAY_EVENTS).min(MAX_PROJECTION_ENTRIES))
            .map(|index| {
                let run = RunBinding::new(
                    base.scope().clone(),
                    base.runner(),
                    base.runner_epoch(),
                    base.chat(),
                    RunId::from_bytes((index as u128).to_le_bytes()),
                    base.epoch(),
                );
                prompt_event(
                    &run,
                    "row",
                    CommandId::from_bytes((index as u128).to_le_bytes()),
                )
            })
            .collect();
        projection.apply(&fixture.page(events)).unwrap();
    }
    assert_eq!(
        projection.transcript().entries.len(),
        MAX_PROJECTION_ENTRIES
    );
    let before = state(&projection);
    let run = RunBinding::new(
        base.scope().clone(),
        base.runner(),
        base.runner_epoch(),
        base.chat(),
        RunId::from_bytes([99; 16]),
        base.epoch(),
    );
    assert_eq!(
        projection
            .apply(&fixture.page(vec![prompt(&run, 99)]))
            .unwrap_err(),
        ProjectionError::Capacity
    );
    assert_eq!(state(&projection), before);
}

#[test]
fn unknown_and_partial_usage_replace_cumulative_snapshots_and_reset_for_a_new_run() {
    let mut fixture = ProjectionFixture::new();
    let first = run_binding(1);
    let next = run_binding(2);
    let mut projection = ConversationProjection::new(fixture.binding().clone());
    projection
        .apply(&fixture.page(vec![prompt(&first, 1)]))
        .unwrap();
    let full = UsageObservation {
        input_tokens: Some(11),
        output_tokens: Some(7),
        total_tokens: Some(18),
        cached_input_tokens: Some(3),
        thought_tokens: Some(2),
        cost_micros: Some(5),
    };
    let zero = UsageObservation {
        input_tokens: Some(0),
        output_tokens: Some(0),
        total_tokens: Some(0),
        cached_input_tokens: Some(0),
        thought_tokens: Some(0),
        cost_micros: Some(0),
    };
    let partial = UsageObservation {
        output_tokens: Some(10),
        ..UsageObservation::default()
    };
    for usage in [full, UsageObservation::default(), full, partial, zero] {
        projection
            .apply(&fixture.page(vec![CoordinatorEvent::Progress {
                run: first.clone(),
                event: EngineEvent::Usage(usage),
            }]))
            .unwrap();
        assert_eq!(projection.usage(), usage);
        match (usage.input_tokens, usage.output_tokens, usage.total_tokens) {
            (Some(input), Some(output), Some(total)) => {
                let tokens = projection.transcript().tokens.as_ref().unwrap();
                assert_eq!(
                    (tokens.input, tokens.output, tokens.total),
                    (input, output, total)
                );
            }
            _ => assert!(projection.transcript().tokens.is_none()),
        }
    }
    // A queued prompt does not change the active run's actual known zero.
    projection
        .apply(&fixture.page(vec![prompt(&next, 2)]))
        .unwrap();
    assert_eq!(projection.usage(), zero);
    projection
        .apply(&fixture.page(vec![
            status(&first, RunStatus::Completed),
            status(&next, RunStatus::Running),
        ]))
        .unwrap();
    assert!(projection.transcript().tokens.is_none());
    assert_eq!(projection.usage(), UsageObservation::default());
    projection
        .apply(&fixture.page(vec![CoordinatorEvent::Progress {
            run: next,
            event: EngineEvent::Usage(partial),
        }]))
        .unwrap();
    assert_eq!(projection.usage(), partial);
    assert!(projection.transcript().tokens.is_none());
}

fn terminal_exec_intent(run: &RunBinding) -> (TerminalCall, ToolIntent) {
    let call = TerminalCall::ExecCommand(nocterm_ai::ExecCommand {
        terminal_id: "terminal".into(),
        program: "true".into(),
        args: vec![],
        stdin: None,
        timeout_ms: None,
        yield_ms: None,
    });
    let intent = ToolIntent::new(
        IntentId::from_bytes([40; 16]),
        StepId::from_bytes([40; 16]),
        execution_authority(run.clone()),
        Fingerprint::from_bytes([40; 32]),
        &call,
    )
    .unwrap();
    (call, intent)
}

fn exit_observation(
    original: &RunBinding,
    intent: &ToolIntent,
    step: StepId,
) -> crate::coordinator::BackgroundObservation {
    use crate::{
        coordinator::{BackgroundObservation, ObservedBackground},
        journal::BackgroundRecordOutcome,
        run::CommitStamp,
    };
    let job = ObservedBackground::new(
        intent.id(),
        intent.authority().clone(),
        "terminal".into(),
        "original-job".into(),
        None,
    )
    .unwrap();
    BackgroundObservation::new(
        job,
        step,
        intent.arguments(),
        intent.mutates(),
        CommitStamp::new(
            original.clone(),
            WriterEpoch::new(1).unwrap(),
            Revision::from_u64(3),
        ),
        BackgroundRecordOutcome::ObservedExit(7),
    )
    .unwrap()
}

#[test]
fn descriptive_background_after_new_run_and_delete_preserves_original_unknown_result() {
    use crate::journal::BackgroundRecordOutcome;
    let mut fixture = ProjectionFixture::new();
    let original = run_binding(1);
    let next = run_binding(2);
    let mut projection = ConversationProjection::new(fixture.binding().clone());
    let (call, intent) = terminal_exec_intent(&original);
    let display = ToolDisplay::new(&call, None);
    projection
        .apply(&fixture.page(vec![
            prompt(&original, 1),
            status(&original, RunStatus::Running),
            CoordinatorEvent::ToolIntent {
                intent: intent.clone(),
                call,
                display: display.clone(),
            },
            CoordinatorEvent::ToolResult {
                run: original.clone(),
                observation: ToolObservation::failed(
                    intent.id(),
                    BoundedJson::new(json!({"error":"unknown"})).unwrap(),
                    display,
                    ToolEffectStatus::Unknown,
                ),
            },
            status(&original, RunStatus::Interrupted),
            prompt(&next, 2),
            status(&next, RunStatus::Running),
            CoordinatorEvent::Deleted {
                command: CommandId::from_bytes([3; 16]),
            },
        ]))
        .unwrap();
    let entries = serde_json::to_value(&projection.transcript().entries).unwrap();
    let control = projection.control();
    let fact = |step| exit_observation(&original, &intent, step);
    let good = fixture.page(vec![CoordinatorEvent::BackgroundLifecycle(Box::new(fact(
        intent.step(),
    )))]);
    let mut altered = good.events().to_vec();
    altered[0].event =
        CoordinatorEvent::BackgroundLifecycle(Box::new(fact(StepId::from_bytes([99; 16]))));
    let bad = ProjectionBatch::new(good.binding().clone(), altered, good.known_head()).unwrap();
    assert_eq!(projection.apply(&bad), Err(ProjectionError::InvalidRecord));
    projection.apply(&good).unwrap();
    assert_eq!(projection.control(), control);
    assert_eq!(projection.phase(), ConversationPhase::Deleted);
    assert_eq!(projection.run(), Some(&next));
    assert_eq!(
        serde_json::to_value(&projection.transcript().entries).unwrap(),
        entries
    );
    let observed = projection.background_observations().collect::<Vec<_>>();
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].job().execution().run(), &original);
    assert_eq!(
        observed[0].outcome(),
        BackgroundRecordOutcome::ObservedExit(7)
    );
}

#[test]
fn approval_observations_preserve_exact_card_and_refuse_changed_resolution_atomically() {
    use crate::coordinator::{ApprovalAnswer, ApprovalChoice};
    let mut fixture = ProjectionFixture::new();
    let run = run_binding(1);
    let mut projection = ConversationProjection::new(fixture.binding().clone());
    let (intent, mut events) = approval_events(&run, 7, Timestamp::from_millis(20_000));
    events.insert(0, prompt(&run, 1));
    projection.apply(&fixture.page(events)).unwrap();
    let key = projection.approvals().next().unwrap().key();
    let card = projection.approval(key).unwrap();
    assert_eq!(card.intent(), &intent);
    assert_eq!(
        card.display().destination.as_deref(),
        Some("original target")
    );
    let request = card.request(ApprovalChoice::Approve).unwrap();
    assert!(key.matches_request(&request));
    let before = state(&projection);
    let answer = ApprovalAnswer::new(
        intent.id(),
        intent.authority().clone(),
        intent.arguments(),
        ApprovalChoice::Approve,
    );
    let valid = fixture.page(vec![CoordinatorEvent::ApprovalResolved {
        command: CommandId::from_bytes([8; 16]),
        answer: answer.clone(),
    }]);
    let mut wrong = valid.events().to_vec();
    wrong[0].event = CoordinatorEvent::ApprovalResolved {
        command: CommandId::from_bytes([8; 16]),
        answer: ApprovalAnswer::new(
            intent.id(),
            intent.authority().clone(),
            Fingerprint::from_bytes([9; 32]),
            ApprovalChoice::Approve,
        ),
    };
    let wrong = ProjectionBatch::new(fixture.binding().clone(), wrong, valid.known_head()).unwrap();
    assert_eq!(
        projection.apply(&wrong),
        Err(ProjectionError::InvalidRecord)
    );
    assert_eq!(state(&projection), before);
    assert!(projection.approval(key).is_some());
    assert_eq!(
        projection.apply(&valid).unwrap(),
        [CommandId::from_bytes([8; 16])]
    );
    assert!(projection.approvals().next().is_none());
    assert!(
        matches!(&projection.transcript().entries[1],Entry::Tool(record) if record.status==ToolStatus::Running)
    );
}

#[test]
fn approval_cards_preserve_expired_facts_and_retire_finite_pending_on_actual_stop() {
    let mut fixture = ProjectionFixture::new();
    let run = run_binding(1);
    let mut projection = ConversationProjection::new(fixture.binding().clone());
    projection
        .apply(&fixture.page(vec![prompt(&run, 1)]))
        .unwrap();
    let (_, expired) = approval_events(&run, 2, Timestamp::from_millis(1));
    let mut historical = projection.clone();
    let mut history = fixture.clone();
    historical.apply(&history.page(expired)).unwrap();
    assert_eq!(
        historical.approvals().next().unwrap().key().expires(),
        Timestamp::from_millis(1)
    );
    let (_, malformed) = approval_events(&run, 2, Timestamp::from_millis(0));
    let mut rejected = fixture.clone();
    let before = state(&projection);
    assert_eq!(
        projection.apply(&rejected.page(malformed)),
        Err(ProjectionError::InvalidRecord)
    );
    assert_eq!(state(&projection), before);
    assert!(projection.approvals().next().is_none());
    for tag in 1..=MAX_PENDING_COMMANDS as u8 {
        let (_, events) = approval_events(&run, tag, Timestamp::from_millis(1_000_000));
        projection.apply(&fixture.page(events)).unwrap();
    }
    assert_eq!(projection.approvals().count(), MAX_PENDING_COMMANDS);
    let mut rejected = fixture.clone();
    let before = state(&projection);
    let (_, extra) = approval_events(&run, 99, Timestamp::from_millis(1_000_000));
    assert_eq!(
        projection.apply(&rejected.page(extra)),
        Err(ProjectionError::Capacity)
    );
    assert_eq!(state(&projection), before);
    assert_eq!(projection.approvals().count(), MAX_PENDING_COMMANDS);
    projection
        .apply(&fixture.page(vec![status(&run, RunStatus::Stopped)]))
        .unwrap();
    assert!(projection.approvals().next().is_none());
}

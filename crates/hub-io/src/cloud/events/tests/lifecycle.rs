//! Background lifecycle keeps the original already committed dispatch.
use super::*;
use nocterm_cloud_model::{
    coordinator::ToolObservation,
    payload::BoundedJson,
    run::{CommitStamp, RunState, ToolIntent},
    tool::{ArgumentFingerprint, BackgroundCommand, ToolEffectStatus},
};
use nocterm_hub_model::cloud::events::{BackgroundOutcome, ObservedEffect};

#[test]
fn runner_events_background_and_unknown_results_keep_original_effect_without_a_new_control_revision()
 {
    let fixture = Fixture::new();
    let encoder = fixture.encoder();
    let work = Arc::new(Fixture::budget(8 * 1024 * 1024));
    let fingerprints = RunnerArgumentFingerprint::new(work.clone());
    let call = super::tools::calls()
        .into_iter()
        .map(|(call, _)| call)
        .find(|call| call.name() == "exec_command")
        .unwrap();
    let intent = ToolIntent::new(
        IntentId::from_bytes([21; 16]),
        StepId::from_bytes([22; 16]),
        fixture.authority(),
        fingerprints.fingerprint(&call).unwrap(),
        &call,
    )
    .unwrap();
    let mut state = RunState::new(
        fixture.run.clone(),
        fixture.authority().session().target().clone(),
        fixture.lease.epoch(),
        Revision::default(),
    );
    state
        .prepare_intent(intent.clone(), Revision::default())
        .unwrap();
    state
        .intent_committed(
            intent.id(),
            CommitStamp::new(
                fixture.run.clone(),
                fixture.lease.epoch(),
                Revision::from_u64(1),
            ),
        )
        .unwrap();
    let dispatch = state
        .dispatch_committed(
            intent.id(),
            CommitStamp::new(
                fixture.run.clone(),
                fixture.lease.epoch(),
                Revision::from_u64(2),
            ),
        )
        .unwrap();
    let command =
        BackgroundCommand::new(dispatch, "terminal".into(), "actual-owned-job".into()).unwrap();
    let next = JournalPosition::default().next(1, 1, false).unwrap();
    for outcome in [
        BackgroundRecordOutcome::Retained,
        BackgroundRecordOutcome::ObservedExit(7),
        BackgroundRecordOutcome::Unknown,
    ] {
        let publication = JournalEventEncoder::encode(
            &encoder,
            &fixture.lease,
            next,
            &command,
            outcome,
            work.as_ref(),
        )
        .unwrap();
        let context = crate::cloud::runner_transport::record_context(
            publication.stream(),
            publication.key_epoch(),
            publication.anchor(),
        )
        .unwrap();
        let public = fixture.identity.public_identity().unwrap();
        let opened = open_event(
            &fixture.key,
            public.signing_key(),
            &context,
            publication.sealed().as_bytes(),
        )
        .unwrap();
        let record = opened.record().unwrap();
        let event = CloudEvent::from_record(&record).unwrap();
        assert_eq!(event.control_revision, 0);
        let CloudEventPayload::BackgroundLifecycle(fact) = event.payload else {
            panic!("wrong event")
        };
        assert_eq!(fact.job.execution.run.id, *fixture.run.run().as_bytes());
        assert_eq!(fact.job.command, "actual-owned-job");
        assert_eq!(fact.dispatch_revision, 2);
        assert!(fact.job.window.is_none());
        assert_eq!(
            fact.outcome,
            match outcome {
                BackgroundRecordOutcome::Retained => BackgroundOutcome::Retained,
                BackgroundRecordOutcome::ObservedExit(exit) =>
                    BackgroundOutcome::ObservedExit(exit),
                BackgroundRecordOutcome::Unknown => BackgroundOutcome::Unknown,
            }
        );
    }
    unknown_result(&fixture, &encoder, intent, &call, work.as_ref());
}
fn unknown_result(
    fixture: &Fixture,
    encoder: &RunnerEventEncoder,
    intent: ToolIntent,
    call: &nocterm_ai::TerminalCall,
    work: &dyn PublicationWorkBudget,
) {
    let output = BoundedJson::new(serde_json::json!({"error":"outcome unknown"})).unwrap();
    let observation = ToolObservation::failed(
        intent.id(),
        output,
        nocterm_ai::tool_display::ToolDisplay::new(call, Some("original".into())),
        ToolEffectStatus::Unknown,
    );
    let event = CoordinatorEvent::ToolResult {
        run: fixture.run.clone(),
        observation,
    };
    let next = JournalPosition::default().next(1, 1, false).unwrap();
    let publication =
        CoordinatorEventEncoder::encode(encoder, &fixture.lease, next, &event, work).unwrap();
    let context = crate::cloud::runner_transport::record_context(
        publication.stream(),
        publication.key_epoch(),
        publication.anchor(),
    )
    .unwrap();
    let public = fixture.identity.public_identity().unwrap();
    let opened = open_event(
        &fixture.key,
        public.signing_key(),
        &context,
        publication.sealed().as_bytes(),
    )
    .unwrap();
    let record = opened.record().unwrap();
    let event = CloudEvent::from_record(&record).unwrap();
    let CloudEventPayload::ToolResult { observation, .. } = event.payload else {
        panic!("wrong event")
    };
    assert!(observation.failed);
    assert!(matches!(observation.effect, ObservedEffect::Unknown));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(observation.output_json).unwrap(),
        serde_json::json!({"error":"outcome unknown"})
    );
}

//! What the runner publishes is what every device shows: each event the encoder seals opens
//! under the runner's pinned key and reduces in the contract device core's transcript.
use super::*;
use nocterm_ai::{
    model::content::{Content, Image},
    tool_display::ToolDisplay,
};
use nocterm_cloud_device::transcript::{Role, ToolStatus, Transcript};
use nocterm_cloud_model::{
    coordinator::{BackgroundObservation, ObservedBackground, ObservedPrompt, ToolObservation},
    engine::EnginePrompt,
    inference::ModelSelection,
    payload::BoundedJson,
    run::{CommitStamp, ToolIntent},
    tool::{ArgumentFingerprint, ToolEffectStatus},
};

#[test]
fn every_published_event_reduces_in_the_device_transcript() {
    let fixture = Fixture::new();
    let encoder = fixture.encoder();
    let work = Arc::new(Fixture::budget(8 * 1024 * 1024));
    let fingerprints = RunnerArgumentFingerprint::new(work.clone());
    let next = JournalPosition::default().next(1, 1, true).unwrap();
    let mut transcript = Transcript::new();
    let mut tools = 0;
    for event in observations(&fixture, &fingerprints) {
        tools += usize::from(matches!(event, CoordinatorEvent::ToolIntent { .. }));
        let publication =
            CoordinatorEventEncoder::encode(&encoder, &fixture.lease, next, &event, work.as_ref())
                .unwrap();
        let context = crate::cloud::runner_transport::record_context(
            publication.stream(),
            publication.key_epoch(),
            publication.anchor(),
        )
        .unwrap();
        let signer = fixture.identity.public_identity().unwrap();
        let opened = open_event(
            &fixture.key,
            signer.signing_key(),
            &context,
            publication.sealed().as_bytes(),
        )
        .unwrap();
        let record = opened.record().unwrap();
        transcript.apply(&CloudEvent::from_record(&record).unwrap());
    }
    let rows = transcript.rows();
    assert_eq!(rows[0].role, Role::User);
    assert_eq!((rows[0].text.as_str(), rows[0].images), ("accepted", 1));
    let calls: Vec<_> = rows.iter().filter_map(|row| row.tool.as_ref()).collect();
    assert_eq!(calls.len(), tools, "one row per announced tool call");
    for call in &calls {
        assert_eq!(call.destination.as_deref(), Some("trusted destination"));
        assert!(
            matches!(call.status, ToolStatus::Unknown | ToolStatus::Failed),
            "{:?}",
            call.status
        );
    }
    // The background job of a command exited with 7.
    assert!(calls.iter().any(|call| call.status == ToolStatus::Failed));
    assert_eq!(transcript.runs().len(), 1);
}

/// The prompt with an image that starts the run.
fn accepted_prompt(fixture: &Fixture) -> CoordinatorEvent {
    CoordinatorEvent::PromptAccepted {
        command: nocterm_cloud_model::journal::CommandAcceptance::new(
            CommandId::from_bytes([31; 16]),
            nocterm_cloud_model::journal::CommandKind::SubmitPrompt,
            Fingerprint::from_bytes([12; 32]),
            nocterm_cloud_model::authority::CommandIssuer::new(
                NamespaceId::new("device").unwrap(),
                Fingerprint::from_bytes([13; 32]),
                Fingerprint::from_bytes([14; 32]),
            ),
            ChatControlRevision::default(),
        ),
        run: fixture.run.clone(),
        request: ObservedPrompt::new(
            EnginePrompt::new(vec![
                Content::text("accepted"),
                Content::Image(Image::new("YWJj", "image/png")),
            ])
            .unwrap(),
            ModelSelection::new(
                NamespaceId::new("provider").unwrap(),
                NamespaceId::new("model").unwrap(),
                NamespaceId::new("price").unwrap(),
            ),
            None,
        ),
    }
}

fn observations(
    fixture: &Fixture,
    fingerprints: &RunnerArgumentFingerprint,
) -> Vec<CoordinatorEvent> {
    let mut events = Vec::new();
    events.push(accepted_prompt(fixture));
    for (index, (call, _)) in super::tools::calls().into_iter().enumerate() {
        let intent = ToolIntent::new(
            IntentId::from_bytes([index as u8; 16]),
            StepId::from_bytes([42; 16]),
            fixture.authority(),
            fingerprints.fingerprint(&call).unwrap(),
            &call,
        )
        .unwrap();
        let display = ToolDisplay::new(&call, Some("trusted destination".into()));
        events.push(CoordinatorEvent::ToolIntent {
            intent: intent.clone(),
            call: call.clone(),
            display: display.clone(),
        });
        events.push(CoordinatorEvent::Approval {
            intent: intent.clone(),
            call: call.clone(),
            display: display.clone(),
            expires: Timestamp::from_millis(1_700_000_060_000),
        });
        events.push(CoordinatorEvent::ToolResult {
            run: fixture.run.clone(),
            observation: ToolObservation::failed(
                intent.id(),
                BoundedJson::new(serde_json::json!({"error":"unknown"})).unwrap(),
                display,
                ToolEffectStatus::Unknown,
            ),
        });
        if matches!(call, nocterm_ai::TerminalCall::ExecCommand(_)) {
            events.push(CoordinatorEvent::BackgroundLifecycle(Box::new(
                BackgroundObservation::new(
                    ObservedBackground::new(
                        intent.id(),
                        fixture.authority(),
                        "terminal".into(),
                        "original-job".into(),
                        None,
                    )
                    .unwrap(),
                    intent.step(),
                    intent.arguments(),
                    intent.mutates(),
                    CommitStamp::new(
                        fixture.run.clone(),
                        fixture.lease.epoch(),
                        Revision::from_u64(2),
                    ),
                    BackgroundRecordOutcome::ObservedExit(7),
                )
                .unwrap(),
            )));
        }
    }
    events
}

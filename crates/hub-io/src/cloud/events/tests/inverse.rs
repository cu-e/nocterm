//! Actual pinned codec roundtrips publish descriptive facts, never run permits.
use super::*;
use nocterm_ai::{
    model::content::{Content, Image},
    tool_display::ToolDisplay,
};
use nocterm_cloud_model::{
    coordinator::{BackgroundObservation, ObservedBackground, ObservedPrompt, ToolObservation},
    engine::EnginePrompt,
    inference::ModelSelection,
    payload::BoundedJson,
    run::{CommitStamp, ToolIntent},
    tool::{ArgumentFingerprint, ToolEffectStatus},
};

#[test]
fn observation_inverse_preserves_native_facts_and_refuses_unadmitted_copies() {
    let fixture = Fixture::new();
    let encoder = fixture.encoder();
    let work = Arc::new(Fixture::budget(8 * 1024 * 1024));
    let fingerprints = RunnerArgumentFingerprint::new(work.clone());
    let events = observations(&fixture, &fingerprints);
    let next = JournalPosition::default().next(1, 1, true).unwrap();
    legacy_reconciliation(&fixture, work.as_ref());
    for event in events {
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
        let allocation = work
            .try_reserve(PublicationAllocation::new(1024 * 1024, 1).unwrap())
            .unwrap();
        let observed = crate::cloud::observations::decode_opened_event(
            fixture.lease.stream(),
            &opened,
            &allocation,
        )
        .unwrap();
        let too_small = work
            .try_reserve(PublicationAllocation::new(8192, 1).unwrap())
            .unwrap();
        assert!(
            crate::cloud::observations::decode_opened_event(
                fixture.lease.stream(),
                &opened,
                &too_small
            )
            .is_err()
        );
        let republished = CoordinatorEventEncoder::encode(
            &encoder,
            &fixture.lease,
            next,
            &observed,
            work.as_ref(),
        )
        .unwrap();
        let second_context = crate::cloud::runner_transport::record_context(
            republished.stream(),
            republished.key_epoch(),
            republished.anchor(),
        )
        .unwrap();
        let second = open_event(
            &fixture.key,
            signer.signing_key(),
            &second_context,
            republished.sealed().as_bytes(),
        )
        .unwrap();
        assert_eq!(
            opened.record().unwrap().payload(),
            second.record().unwrap().payload()
        );
    }
}

fn observations(
    fixture: &Fixture,
    fingerprints: &RunnerArgumentFingerprint,
) -> Vec<CoordinatorEvent> {
    let mut events = Vec::new();
    let mut prompt = nocterm_cloud_model::projection::testing::prompt_event(
        &fixture.run,
        "accepted",
        CommandId::from_bytes([31; 16]),
    );
    if let CoordinatorEvent::PromptAccepted { request, .. } = &mut prompt {
        *request = ObservedPrompt::new(
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
        );
    }
    events.push(prompt);
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

fn legacy_reconciliation(fixture: &Fixture, work: &dyn PublicationWorkBudget) {
    use nocterm_hub_model::cloud::{events::CloudEventPayload, payload::ApprovalChoice};
    let next = JournalPosition::default().next(1, 1, true).unwrap();
    let publication = CoordinatorEventEncoder::encode(
        &fixture.encoder(),
        &fixture.lease,
        next,
        &CoordinatorEvent::Status {
            run: fixture.run.clone(),
            status: RunStatus::Interrupted,
        },
        work,
    )
    .unwrap();
    let context = crate::cloud::runner_transport::record_context(
        publication.stream(),
        publication.key_epoch(),
        publication.anchor(),
    )
    .unwrap();
    for choice in [
        ApprovalChoice::ReconcileSucceeded,
        ApprovalChoice::ReconcileFailed,
        ApprovalChoice::ReconcileKnown {
            result_digest: [47; 32],
        },
    ] {
        let body = CloudEvent {
            control_revision: next.control().get(),
            timestamp_ms: 1_700_000_000_000,
            payload: CloudEventPayload::ApprovalResolved {
                command: [44; 16],
                answer: Box::new(nocterm_hub_model::cloud::payload::ApprovalPayload {
                    intent: [45; 16],
                    execution: crate::cloud::events::values::execution(&fixture.authority())
                        .unwrap(),
                    arguments_digest: [46; 32],
                    choice,
                }),
            },
        }
        .canonical_bytes()
        .unwrap();
        let sealed = fixture
            .material
            .with_material(fixture.lease.stream(), |key, _, signer| {
                nocterm_keys::cloud::seal_event(key, signer, context.clone(), &body)
                    .map_err(crate::cloud::events::encoding_error)
            })
            .unwrap();
        let public = fixture.identity.public_identity().unwrap();
        let opened = open_event(&fixture.key, public.signing_key(), &context, &sealed).unwrap();
        let allocation = work
            .try_reserve(PublicationAllocation::new(1024 * 1024, 1).unwrap())
            .unwrap();
        let result = crate::cloud::observations::decode_opened_event(
            fixture.lease.stream(),
            &opened,
            &allocation,
        );
        if matches!(choice, ApprovalChoice::ReconcileKnown { .. }) {
            let CoordinatorEvent::ApprovalResolved { answer, .. } = result.unwrap() else {
                panic!("missing descriptive answer")
            };
            assert_eq!(
                answer.choice(),
                nocterm_cloud_model::coordinator::ApprovalChoice::ReconcileKnown(
                    Fingerprint::from_bytes([47; 32])
                )
            );
        } else {
            assert!(matches!(
                result,
                Err(crate::cloud::observations::ObservationError::Projection(
                    nocterm_cloud_model::projection::ProjectionError::Unsupported
                ))
            ));
        }
    }
}

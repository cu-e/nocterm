//! Actual SDK records supply every baseline and late historical fact.
use super::super::{
    sealing::{RecordPayload, RecordSealer},
    snapshot::SnapshotProjection,
    values,
};
use super::*;
use nocterm_cloud_model::journal::{
    HeadAnchor, HeadAnchorKind, HeadRecordPosition, JournalCipher, OutboxEntry, SnapshotLimits,
    SnapshotProjectionSource,
};
use nocterm_hub_model::cloud::events::{
    BackgroundJob, BackgroundLifecyclePayload, BackgroundOutcome, CloudSnapshotPayload,
};
use nocterm_keys::cloud::{RunnerSigningKey, open_snapshot, seal_event};

const TIME: u64 = 1_700_000_000_000;
fn limits() -> SnapshotLimits {
    SnapshotLimits::new(4096, 16 * 1024 * 1024, 8 * 1024 * 1024).unwrap()
}
fn source(
    fixture: &Fixture,
    covered: JournalPosition,
    previous: Option<ResidentPublication>,
    records: Vec<ResidentPublication>,
) -> SnapshotProjectionSource {
    SnapshotProjectionSource::new(
        fixture.lease.stream().clone(),
        fixture
            .material
            .chat_key_epoch(fixture.lease.stream())
            .unwrap(),
        covered,
        previous,
        records,
        limits(),
    )
    .unwrap()
}
fn check_snapshot(
    fixture: &Fixture,
    publication: &ResidentPublication,
    check: impl FnOnce(&CloudSnapshotPayload<'_>),
) {
    let context = crate::cloud::runner_transport::record_context(
        publication.stream(),
        publication.key_epoch(),
        publication.anchor(),
    )
    .unwrap();
    assert!(context.run().is_none());
    let opened = open_snapshot(
        &fixture.key,
        fixture.identity.public_identity().unwrap().signing_key(),
        &context,
        publication.sealed().as_bytes(),
    )
    .unwrap();
    let signed = opened.record().unwrap();
    check(&CloudSnapshotPayload::from_record(&signed).unwrap());
}
mod history;
mod window;
use history::History;
fn newer_run(fixture: &Fixture) -> RunBinding {
    RunBinding::new(
        fixture.run.scope().clone(),
        fixture.run.runner(),
        fixture.run.runner_epoch(),
        fixture.run.chat(),
        RunId::from_bytes([71; 16]),
        RunEpoch::new(72).unwrap(),
    )
}
#[test]
fn runner_snapshots_merge_actual_baseline_and_late_historical_unknown_without_rewriting_facts() {
    let fixture = Fixture::new();
    let mut history = History::new(&fixture);
    history.prompt(&fixture.run, 1);
    history.usage(&fixture.run, None);
    let tool = history.tool(&fixture.run, 21);
    history.background_result(&tool);
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
    let newer = newer_run(&fixture);
    history.prompt(&newer, 2);
    history.status(&newer, RunStatus::Running);
    history.background(&tool, BackgroundOutcome::Unknown);
    let input = history.source(Some(baseline));
    let snapshot = projection
        .encode(
            &fixture.lease,
            history.next_snapshot(),
            &input,
            limits(),
            history.time(),
        )
        .unwrap();
    check_snapshot(&fixture, &snapshot, |body| {
        assert!(!body.omitted_earlier);
        assert_eq!(body.events.len(), 8);
        let CloudEventPayload::Usage { run, usage } = body.events[1].payload else {
            panic!("usage changed")
        };
        assert_eq!(run.id, *fixture.run.run().as_bytes());
        assert_eq!(usage.cost_micros, None);
        assert_eq!(usage.input_tokens, Some(7));
        let CloudEventPayload::PromptAccepted { run, .. } = body.events[5].payload else {
            panic!("new prompt missing")
        };
        assert_eq!(run.id, *newer.run().as_bytes());
        let CloudEventPayload::BackgroundLifecycle(fact) = &body.events[7].payload else {
            panic!("fact missing")
        };
        assert_eq!(fact.job.execution.run.id, *fixture.run.run().as_bytes());
        assert_eq!(fact.outcome, BackgroundOutcome::Unknown);
        assert_eq!(fact.dispatch_revision, tool.dispatch_revision);
        assert_eq!(body.events[7].timestamp_ms, history.time());
        assert_eq!(body.control_revision, input.covered().control().get());
    });
}
fn two_events(fixture: &Fixture) -> (JournalPosition, SnapshotProjectionSource, u64) {
    let mut history = History::new(fixture);
    history.prompt(&fixture.run, 1);
    history.text(&fixture.run, &"large optional display ".repeat(100));
    history.status(&fixture.run, RunStatus::Completed);
    (
        history.next_snapshot(),
        history.source(None),
        history.time(),
    )
}
#[test]
fn runner_snapshots_keep_causal_roots_under_actual_ciphertext_and_record_limits() {
    let fixture = Fixture::new();
    let (next, input, time) = two_events(&fixture);
    let work = Fixture::budget(256 * 1024 * 1024);
    let projection = SnapshotProjection::new(&fixture.material, &work);
    let snapshot = projection
        .encode(&fixture.lease, next, &input, limits(), time)
        .unwrap();
    let exact = two_root_ciphertext_limit(&fixture, &input, next, time, &work);
    let small = projection
        .encode(
            &fixture.lease,
            next,
            &input,
            SnapshotLimits::new(4096, 16 * 1024 * 1024, exact).unwrap(),
            time,
        )
        .unwrap();
    assert_eq!(small.sealed().len(), exact);
    check_snapshot(&fixture, &small, |body| {
        assert!(body.omitted_earlier);
        assert_eq!(body.events.len(), 2);
        assert!(matches!(
            body.events[0].payload,
            CloudEventPayload::PromptAccepted { .. }
        ));
        assert!(matches!(
            body.events[1].payload,
            CloudEventPayload::Status { .. }
        ));
    });
    let previous = source(&fixture, next, Some(snapshot), Vec::new());
    let after = next.next(1, 1, false).unwrap();
    let two = SnapshotLimits::new(2, 16 * 1024 * 1024, 8 * 1024 * 1024).unwrap();
    let selected = projection
        .encode(&fixture.lease, after, &previous, two, time)
        .unwrap();
    check_snapshot(&fixture, &selected, |body| {
        assert!(body.omitted_earlier);
        assert_eq!(body.events.len(), 2);
    });
    assert!(matches!(
        projection.encode(
            &fixture.lease,
            after,
            &previous,
            SnapshotLimits::new(1, 16 * 1024 * 1024, 8 * 1024 * 1024).unwrap(),
            time
        ),
        Err(JournalError::Capacity)
    ));
}
fn two_root_ciphertext_limit(
    fixture: &Fixture,
    input: &SnapshotProjectionSource,
    next: JournalPosition,
    time: u64,
    work: &dyn PublicationWorkBudget,
) -> usize {
    let mut roots = 0;
    for original in [
        input.records().first().unwrap(),
        input.records().last().unwrap(),
    ] {
        let context = crate::cloud::runner_transport::record_context(
            original.stream(),
            original.key_epoch(),
            original.anchor(),
        )
        .unwrap();
        let opened = open_event(
            &fixture.key,
            fixture.identity.public_identity().unwrap().signing_key(),
            &context,
            original.sealed().as_bytes(),
        )
        .unwrap();
        roots += CloudEvent::from_record(&opened.record().unwrap())
            .unwrap()
            .canonical_len()
            .unwrap();
    }
    let empty = CloudSnapshotPayload {
        control_revision: next.control().get(),
        timestamp_ms: time,
        omitted_earlier: true,
        events: Vec::new(),
    }
    .canonical_len()
    .unwrap();
    let capacity = RecordSealer::new(&fixture.material, work)
        .snapshot_body_capacity(&fixture.lease, next, limits().output())
        .unwrap();
    limits().output() - capacity + empty + roots
}

#[test]
fn runner_snapshots_refuse_capacity_source_time_or_position_changes() {
    let fixture = Fixture::new();
    let (next, input, time) = two_events(&fixture);
    let work = Fixture::budget(256 * 1024 * 1024);
    let projection = SnapshotProjection::new(&fixture.material, &work);
    assert!(matches!(
        projection.encode(
            &fixture.lease,
            next,
            &input,
            SnapshotLimits::new(1, 1024 * 1024, 1024 * 1024).unwrap(),
            time
        ),
        Err(JournalError::Capacity)
    ));
    assert!(matches!(
        SnapshotProjection::new(&fixture.material, &Fixture::budget(1)).encode(
            &fixture.lease,
            next,
            &input,
            limits(),
            time
        ),
        Err(JournalError::Capacity)
    ));
    assert!(
        projection
            .encode(
                &fixture.lease,
                next,
                &input,
                SnapshotLimits::new(4096, 16 * 1024 * 1024, 1).unwrap(),
                time
            )
            .is_err()
    );
    assert!(
        projection
            .encode(&fixture.lease, next, &input, limits(), TIME - 1)
            .is_err()
    );
    assert!(
        projection
            .encode(
                &fixture.lease,
                next.next(1, 1, false).unwrap(),
                &input,
                limits(),
                time
            )
            .is_err()
    );
    let other = another_writer(&fixture);
    assert!(matches!(
        projection.encode(&other, next, &input, limits(), time),
        Err(JournalError::Fenced)
    ));
    let snapshot = projection
        .encode(&fixture.lease, next, &input, limits(), time)
        .unwrap();
    let previous = source(&fixture, next, Some(snapshot), Vec::new());
    assert!(
        projection
            .encode(
                &fixture.lease,
                next.next(1, 1, false).unwrap(),
                &previous,
                limits(),
                time - 1
            )
            .is_err()
    );
    fixture.material.retire();
    assert!(
        projection
            .encode(&fixture.lease, next, &input, limits(), time)
            .is_err()
    );
}

fn another_writer(fixture: &Fixture) -> WriterLease {
    WriterLease::new(
        nocterm_cloud_model::authority::StreamBinding::new(
            fixture.run.scope().clone(),
            fixture.run.runner(),
            fixture.run.runner_epoch(),
            CloudChatId::from_bytes([99; 16]),
        ),
        fixture.lease.epoch(),
    )
}

fn replaced(
    fixture: &Fixture,
    original: &ResidentPublication,
    mutation: usize,
) -> ResidentPublication {
    let work = Fixture::budget(1024 * 1024);
    let allocation = work
        .try_reserve(
            PublicationAllocation::new(
                ResidentPublication::required_bytes(original.sealed().len() + 1024).unwrap(),
                1,
            )
            .unwrap(),
        )
        .unwrap();
    let mut sealed = original.sealed().as_bytes().to_vec();
    let mut id = original.id();
    if mutation == 0 {
        *sealed.last_mut().unwrap() ^= 1;
    }
    if mutation == 1 {
        id = EventId::from_bytes([100; 16]);
    }
    if mutation == 2 {
        let context = crate::cloud::runner_transport::record_context(
            original.stream(),
            original.key_epoch(),
            original.anchor(),
        )
        .unwrap();
        let opened = open_event(
            &fixture.key,
            fixture.identity.public_identity().unwrap().signing_key(),
            &context,
            &sealed,
        )
        .unwrap();
        let signed = opened.record().unwrap();
        sealed = seal_event(
            &fixture.key,
            &RunnerSigningKey::generate().unwrap(),
            context,
            signed.payload(),
        )
        .unwrap();
    }
    let anchor = HeadAnchor::new(
        HeadAnchorKind::Event,
        if mutation == 3 {
            HeadRecordPosition::new(
                original.anchor().position().writer(),
                original.sequence(),
                original.record(),
                StreamRevision::from_u64(99),
            )
            .unwrap()
        } else {
            original.anchor().position()
        },
        id,
        original.anchor().run().cloned(),
        Fingerprint::from_bytes(Sha256::digest(&sealed).into()),
    );
    ResidentPublication::new(
        original.stream().clone(),
        OutboxEntry::new(
            anchor,
            original.key_epoch(),
            original.delivery_id(),
            nocterm_cloud_model::payload::OpaqueBytes::new(sealed).unwrap(),
        ),
        allocation,
    )
    .unwrap()
}
#[test]
fn runner_snapshots_refuse_rehashed_tamper_wrong_signer_context_relabel_and_gaps() {
    let fixture = Fixture::new();
    let mut history = History::new(&fixture);
    history.prompt(&fixture.run, 1);
    history.status(&fixture.run, RunStatus::Completed);
    let base = history.source(None);
    let prompt = base.records()[0].clone();
    let original = base.records()[1].clone();
    let work = Fixture::budget(256 * 1024 * 1024);
    let projection = SnapshotProjection::new(&fixture.material, &work);
    assert!(
        projection
            .encode(
                &fixture.lease,
                history.next_snapshot(),
                &base,
                limits(),
                history.time()
            )
            .is_ok()
    );
    for mutation in 0..4 {
        let altered = replaced(&fixture, &original, mutation);
        let input = source(
            &fixture,
            base.covered(),
            None,
            vec![prompt.clone(), altered],
        );
        assert!(
            projection
                .encode(
                    &fixture.lease,
                    history.next_snapshot(),
                    &input,
                    limits(),
                    history.time()
                )
                .is_err(),
            "mutation {mutation}"
        );
    }
    assert!(
        SnapshotProjectionSource::new(
            fixture.lease.stream().clone(),
            original.key_epoch(),
            base.covered(),
            None,
            vec![original],
            limits()
        )
        .is_err()
    );
}

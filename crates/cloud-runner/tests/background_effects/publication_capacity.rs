//! Already admitted terminal facts keep their reserved publication capacity.
use super::*;
use nocterm_cloud_model::coordinator::{
    CommandReceiveBudget, IntakeLane, IntakeLimits, IntakePoolLimits,
};
use nocterm_cloud_runner::CoordinatorPublicationBudgets;

#[test]
fn reserved_publication_finishes_original_job_after_tombstone_with_both_command_pools_full() {
    let root = private_root();
    let receive = CommandReceiveBudget::new(IntakeLimits::default());
    let work = CoordinatorPublicationBudgets::new(
        receive.clone(),
        IntakePoolLimits::new(256 * 1024, 4, 4).unwrap(),
    );
    let store = journal(root.path(), Arc::new(TestCipher::default()))
        .with_background_events(Arc::new(TestEvents::default()), work)
        .unwrap();
    let original = claim(&store, None);
    let mut state = state(1, &original);
    let mut position = JournalPosition::default();
    let job = dispatch(&store, &original, &mut state, &mut position, 1);
    block_on(store.retain(job.clone())).unwrap();
    delete_chat(&store, &original);
    let cleanup = store.cleanup();
    let current = block_on(
        cleanup.claim_cleanup(
            WriterClaim::new(
                stream(),
                Some(original.epoch()),
                original.epoch().next().unwrap(),
            )
            .unwrap(),
        ),
    )
    .unwrap();
    let before = block_on(store.head(stream())).unwrap().unwrap();
    let ordinary = receive
        .try_reserve(
            IntakeLane::Ordinary,
            receive.limits(IntakeLane::Ordinary).bytes(),
            0,
        )
        .unwrap();
    let urgent = receive
        .try_reserve(
            IntakeLane::Urgent,
            receive.limits(IntakeLane::Urgent).bytes(),
            0,
        )
        .unwrap();
    block_on(cleanup.finish_background(
        current.clone(),
        job.clone(),
        BackgroundCommandOutcome::Unknown,
    ))
    .unwrap();
    assert_eq!(pending(&store).len(), 1);
    block_on(cleanup.finish_background(
        current.clone(),
        job.clone(),
        BackgroundCommandOutcome::ObservedExit(0),
    ))
    .unwrap();
    assert!(pending(&store).is_empty());
    let after = block_on(store.head(stream())).unwrap().unwrap();
    assert_eq!(after.lease(), current.writer());
    assert_eq!(after.position().control(), before.position().control());
    assert!(after.deleted());
    assert_eq!(
        after.position().events(),
        before.position().events().advance(2).unwrap()
    );
    let page = block_on(store.read(stream(), before.position().records(), limits())).unwrap();
    assert_eq!(
        page.records().last().unwrap().record().context().run(),
        Some(job.dispatch().intent().authority().run())
    );
    assert_eq!(
        receive.usage(IntakeLane::Ordinary).unwrap().bytes,
        ordinary.bytes()
    );
    assert_eq!(
        receive.usage(IntakeLane::Urgent).unwrap().bytes,
        urgent.bytes()
    );
}

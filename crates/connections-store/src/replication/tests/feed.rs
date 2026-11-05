//! Malformed or excessive feeds never produce a partial plan or advance the cursor.
use super::*;
use nocterm_connections_model::replica::{RemoteProfile, ReplicaPage};
use std::time::Duration;

fn page(cursor: &str, profiles: Vec<Profile>, more: bool) -> ReplicaPage {
    ReplicaPage {
        changes: profiles
            .into_iter()
            .map(|profile| RemoteProfile {
                id: profile.id,
                scope: ReplicaScope::Personal,
                version: ReplicaVersion::new(1),
                body: RemoteBody::Present(Box::new(profile)),
            })
            .collect(),
        cursor: ReplicaCursor::new(cursor),
        more,
    }
}
fn unchanged(fixture: &Fixture, candidate: &Profile, cx: &gpui::TestAppContext) {
    assert!(fixture.local(cx, candidate).is_none());
    assert!(fixture.replica.writes().is_empty());
    assert!(fixture.persisted_state().cursor().is_none());
}

#[gpui::test]
fn a_feed_that_stalls_with_more_pages_is_rejected_before_any_profile_changes(
    cx: &mut gpui::TestAppContext,
) {
    let candidate = profile("not-applied");
    let fixture = Fixture::new(cx, &[]);
    fixture.replica.script_pages(vec![
        page("first", vec![candidate.clone()], true),
        page("first", vec![], true),
    ]);
    fixture.attach(cx);
    assert!(matches!(
        fixture.status(cx),
        ReplicationStatus::Failed(ReplicationFailure::Feed(ReplicationFeedError::NonProgress))
    ));
    unchanged(&fixture, &candidate, cx);
}

#[gpui::test]
fn a_feed_cycle_is_refused_even_when_its_last_page_claims_completion(
    cx: &mut gpui::TestAppContext,
) {
    let candidate = profile("not-applied");
    let fixture = Fixture::new(cx, &[]);
    fixture.replica.script_pages(vec![
        page("first", vec![candidate.clone()], true),
        page("second", vec![], true),
        page("first", vec![], false),
    ]);
    fixture.attach(cx);
    assert!(matches!(
        fixture.status(cx),
        ReplicationStatus::Failed(ReplicationFailure::Feed(ReplicationFeedError::Cycle))
    ));
    unchanged(&fixture, &candidate, cx);
}

#[gpui::test]
fn excessive_pages_stop_requesting_the_feed_without_advancing_the_cursor(
    cx: &mut gpui::TestAppContext,
) {
    let candidate = profile("not-applied");
    let fixture = Fixture::new(cx, &[]);
    let mut pages = vec![page("first", vec![candidate.clone()], true)];
    pages.extend((1..=256).map(|index| page(&format!("cursor-{index}"), vec![], true)));
    fixture.replica.script_pages(pages);
    fixture.attach(cx);
    assert!(matches!(
        fixture.status(cx),
        ReplicationStatus::Failed(ReplicationFailure::Feed(ReplicationFeedError::Pages))
    ));
    assert_eq!(fixture.replica.calls().len(), 256);
    unchanged(&fixture, &candidate, cx);
}

#[gpui::test]
fn excessive_change_counts_are_refused_before_planning(cx: &mut gpui::TestAppContext) {
    let candidate = profile("not-applied");
    let fixture = Fixture::new(cx, &[]);
    let change = RemoteProfile {
        scope: ReplicaScope::Personal,
        id: candidate.id,
        version: ReplicaVersion::new(1),
        body: RemoteBody::Deleted,
    };
    fixture.replica.script_pages(vec![ReplicaPage {
        changes: vec![change; 16_385],
        cursor: ReplicaCursor::new("end"),
        more: false,
    }]);
    fixture.attach(cx);
    assert!(matches!(
        fixture.status(cx),
        ReplicationStatus::Failed(ReplicationFailure::Feed(ReplicationFeedError::Changes))
    ));
    unchanged(&fixture, &candidate, cx);
}

#[gpui::test]
fn the_payload_budget_applies_across_pages_before_any_remote_or_local_effect(
    cx: &mut gpui::TestAppContext,
) {
    let candidate = profile("not-applied");
    let fixture = Fixture::new(cx, &[]);
    let mut large = profile("large");
    large.description = "x".repeat(9 * 1024 * 1024);
    let mut second = profile("second");
    second.description = "y".repeat(9 * 1024 * 1024);
    fixture.replica.script_pages(vec![
        page("first", vec![candidate.clone(), large], true),
        page("second", vec![second], false),
    ]);
    fixture.attach(cx);
    assert!(matches!(
        fixture.status(cx),
        ReplicationStatus::Failed(ReplicationFailure::Feed(ReplicationFeedError::Bytes))
    ));
    unchanged(&fixture, &candidate, cx);
}

#[gpui::test]
fn an_oversized_cursor_never_becomes_the_saved_replication_position(cx: &mut gpui::TestAppContext) {
    let candidate = profile("not-applied");
    let fixture = Fixture::new(cx, &[]);
    fixture.replica.script_pages(vec![page(
        &"x".repeat(4097),
        vec![candidate.clone()],
        false,
    )]);
    fixture.attach(cx);
    assert!(matches!(
        fixture.status(cx),
        ReplicationStatus::Failed(ReplicationFailure::Feed(ReplicationFeedError::CursorSize))
    ));
    unchanged(&fixture, &candidate, cx);
}

#[gpui::test]
fn a_stalled_feed_has_a_complete_acquisition_deadline_and_cancels_its_reader(
    cx: &mut gpui::TestAppContext,
) {
    let candidate = profile("not-applied");
    let fixture = Fixture::new(cx, &[]);
    fixture
        .replica
        .script_pages(vec![page("end", vec![candidate.clone()], false)]);
    let release = fixture.replica.hold_next_read();
    fixture.attach(cx);
    assert!(matches!(fixture.status(cx), ReplicationStatus::Syncing));
    cx.executor().advance_clock(Duration::from_secs(121));
    cx.run_until_parked();
    assert!(matches!(
        fixture.status(cx),
        ReplicationStatus::Failed(ReplicationFailure::Feed(ReplicationFeedError::TimedOut))
    ));
    assert!(
        release.send(()).is_err(),
        "the original request was dropped"
    );
    unchanged(&fixture, &candidate, cx);
}

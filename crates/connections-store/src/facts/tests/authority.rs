use super::*;
use nocterm_connections_model::{os::OsId, os_observation::OsObservationError};
use nocterm_session::{FsError, testing::Gate};
#[gpui::test]
fn sessions_are_probed_once_and_chosen_icons_skip_detection(cx: &mut TestAppContext) {
    let fixture = Fixture::memory(false, cx);
    let session = fs();
    fixture.os.answer_next(Ok(OsId::new("debian")));
    fixture.request(session.clone(), cx).unwrap();
    cx.run_until_parked();
    assert_eq!(fixture.raw_os(cx).as_deref(), Some("debian"));
    fixture.request(session, cx).unwrap();
    cx.run_until_parked();
    assert_eq!(fixture.os.calls(), 1);
    let mut chosen = fixture.profile.clone();
    chosen.icon = Some("arch".into());
    fixture.save(chosen, cx);
    fixture.request(fs(), cx).unwrap();
    cx.run_until_parked();
    assert_eq!(fixture.os.calls(), 1);
    fixture.save(fixture.profile.clone(), cx);
    fixture
        .os
        .answer_next(Err(OsObservationError::Home(FsError::NotConnected)));
    let failed = fs();
    fixture.request(failed.clone(), cx).unwrap();
    cx.run_until_parked();
    assert_eq!(fixture.raw_os(cx).as_deref(), Some("debian"));
    fixture.request(failed, cx).unwrap();
    assert_eq!(fixture.os.calls(), 2);
    fixture.os.answer_next(Ok(None));
    fixture.request(fs(), cx).unwrap();
    cx.run_until_parked();
    assert_eq!(fixture.raw_os(cx).as_deref(), Some("debian"));
}
#[gpui::test]
fn held_os_results_cannot_restore_deleted_readded_endpoint_or_manual_history(
    cx: &mut TestAppContext,
) {
    for cycle in 0..3 {
        let fixture = Fixture::memory(false, cx);
        let session = fs();
        let gate = Gate::pending();
        fixture.os.held_next(Ok(OsId::new("debian")), gate.clone());
        fixture.request(session.clone(), cx).unwrap();
        cx.run_until_parked();
        match cycle {
            0 => {
                let task = fixture.connections.update(cx, |connections, cx| {
                    connections.delete_profile(fixture.profile.id, cx)
                });
                cx.run_until_parked();
                futures::executor::block_on(task).unwrap();
                fixture.save(fixture.profile.clone(), cx);
            }
            1 => {
                let mut changed = fixture.profile.clone();
                changed.target.port = 2222;
                fixture.save(changed, cx);
                fixture.save(fixture.profile.clone(), cx);
            }
            _ => {
                let mut chosen = fixture.profile.clone();
                chosen.icon = Some("arch".into());
                fixture.save(chosen, cx);
                fixture.save(fixture.profile.clone(), cx);
            }
        }
        assert_eq!(
            fixture.jobs(cx),
            31,
            "invalidating authority does not free a physical task"
        );
        gate.release();
        cx.run_until_parked();
        assert_eq!(fixture.raw_os(cx), None);
        assert_eq!(fixture.jobs(cx), 32);
        fixture.request(session, cx).unwrap();
        assert_eq!(
            fixture.os.calls(),
            1,
            "same capability keeps its one OS decision"
        );
    }
}
#[gpui::test]
fn newer_original_session_owns_slot_and_old_completion_cannot_clear_it(cx: &mut TestAppContext) {
    let fixture = Fixture::memory(false, cx);
    let (old, new) = (fs(), fs());
    let (old_gate, new_gate) = (Gate::pending(), Gate::pending());
    fixture
        .os
        .held_next(Ok(OsId::new("debian")), old_gate.clone());
    fixture
        .os
        .held_next(Ok(OsId::new("arch")), new_gate.clone());
    fixture.request(old.clone(), cx).unwrap();
    fixture.request(new.clone(), cx).unwrap();
    cx.run_until_parked();
    let recorded = fixture.os.sessions();
    assert!(std::sync::Weak::ptr_eq(&recorded[0], &Arc::downgrade(&old)));
    assert!(std::sync::Weak::ptr_eq(&recorded[1], &Arc::downgrade(&new)));
    old_gate.release();
    cx.run_until_parked();
    assert_eq!(fixture.raw_os(cx), None);
    assert_eq!(fixture.jobs(cx), 31);
    new_gate.release();
    cx.run_until_parked();
    assert_eq!(fixture.raw_os(cx).as_deref(), Some("arch"));
}
#[gpui::test]
fn exact_session_user_port_and_host_must_match_committed_target(cx: &mut TestAppContext) {
    let fixture = Fixture::memory(true, cx);
    for part in 0..3 {
        let mut target = fixture.profile.target.clone();
        match part {
            0 => target.user = "other".into(),
            1 => target.port = 2222,
            _ => target.host = "other".into(),
        };
        let session = fs();
        let result = fixture.facts.update(cx, |facts, cx| {
            facts.request_probe(
                ProbeRequest::new(
                    fixture.profile.id,
                    target,
                    session.clone(),
                    lease(&session, fixture.facts.entity_id(), 0),
                ),
                cx,
            )
        });
        assert!(matches!(
            result,
            Err(crate::FactsAdmissionError::EndpointMismatch)
        ));
    }
    assert_eq!(fixture.os.calls(), 0);
    assert!(fixture.script.calls().is_empty());
    assert_eq!(fixture.jobs(cx), 32);
}

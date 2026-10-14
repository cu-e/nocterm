use super::*;
use nocterm_session::testing::Gate;
#[gpui::test]
fn physical_slots_and_atomic_refusal_leave_capability_decisions_retryable(cx: &mut TestAppContext) {
    let fixture = Fixture::memory(false, cx);
    let mut held = Vec::new();
    let mut sessions = Vec::new();
    for _ in 0..32 {
        let gate = Gate::pending();
        fixture.os.held_next(Ok(None), gate.clone());
        let session = fs();
        fixture.request(session.clone(), cx).unwrap();
        held.push(gate);
        sessions.push(session);
    }
    cx.run_until_parked();
    assert_eq!(fixture.jobs(cx), 0);
    let refused = fs();
    assert!(matches!(
        fixture.request(refused.clone(), cx),
        Err(crate::FactsAdmissionError::Capacity {
            needed: 1,
            available: 0
        })
    ));
    assert_eq!(fixture.os.calls(), 32);
    held[0].release();
    cx.run_until_parked();
    fixture.os.answer_next(Ok(None));
    fixture.request(refused, cx).unwrap();
    cx.run_until_parked();
    assert_eq!(fixture.os.calls(), 33);
    for gate in held {
        gate.release();
    }
    cx.run_until_parked();
    assert_eq!(fixture.jobs(cx), 32);
    drop(sessions);
}
#[gpui::test]
fn authority_revocation_keeps_native_slots_until_real_completion(cx: &mut TestAppContext) {
    let fixture = Fixture::memory(false, cx);
    let mut held = Vec::new();
    let mut sessions = Vec::new();
    for _ in 0..32 {
        let gate = Gate::pending();
        fixture.os.held_next(Ok(None), gate.clone());
        let session = fs();
        fixture.request(session.clone(), cx).unwrap();
        held.push(gate);
        sessions.push(session);
    }
    cx.run_until_parked();
    let mut changed = fixture.profile.clone();
    changed.target.host = "replacement".into();
    fixture.save(changed, cx);
    fixture.facts.update(cx, |facts, cx| {
        facts.synchronize(cx);
    });
    assert_eq!(fixture.jobs(cx), 0);
    for gate in held {
        gate.release();
    }
    cx.run_until_parked();
    assert_eq!(fixture.jobs(cx), 32);
    assert_eq!(fixture.raw_os(cx), None);
    drop(sessions);
}

#[gpui::test]
fn two_native_jobs_are_reserved_atomically_before_capability_markers(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        profile(),
        Default::default(),
        true,
        threaded(cx),
        ready_writer(),
        cx,
    );
    fixture.publish_policy(|policy| policy.enabled = false, cx);
    let mut sessions = Vec::new();
    let mut gates = Vec::new();
    for _ in 0..31 {
        let gate = Gate::pending();
        fixture.os.held_next(Ok(None), gate.clone());
        let session = fs();
        fixture.request(session.clone(), cx).unwrap();
        sessions.push(session);
        gates.push(gate);
    }
    until(cx, |_| fixture.os.calls() == 31);
    assert_eq!(fixture.jobs(cx), 1);
    fixture.publish_policy(|policy| policy.enabled = true, cx);
    let refused = fs();
    assert!(matches!(
        fixture.request(refused.clone(), cx),
        Err(crate::FactsAdmissionError::Capacity {
            needed: 2,
            available: 1
        })
    ));
    cx.run_until_parked();
    assert_eq!(fixture.os.calls(), 31);
    assert!(fixture.script.calls().is_empty());
    gates[0].release();
    until(cx, |cx| fixture.jobs(cx) == 2);
    let os = Gate::pending();
    fixture.os.held_next(Ok(None), os.clone());
    let country = Arc::new(nocterm_connections_model::test_support::BlockingGate::default());
    let _release = country.release_on_drop();
    fixture.country_answer();
    fixture.script.suspend_next_country(country.clone());
    assert!(matches!(
        fixture.request(refused.clone(), cx),
        Ok(ProbeOutcome::Admitted {
            os: true,
            country: true
        })
    ));
    until(cx, |_| fixture.os.calls() == 32 && country.entered() == 1);
    assert_eq!(fixture.jobs(cx), 0);
    for gate in gates {
        gate.release();
    }
    os.release();
    country.release();
    until(cx, |cx| fixture.jobs(cx) == 32);
    assert_eq!(fixture.raw_country(cx).as_deref(), Some("de"));
    drop((sessions, refused));
}

#[gpui::test]
fn flag_and_probe_lanes_take_turns_without_early_physical_slot_release(cx: &mut TestAppContext) {
    let mut server = profile();
    server.country = Some("xx".into());
    let fixture = Fixture::new(
        server,
        Default::default(),
        true,
        threaded(cx),
        ready_writer(),
        cx,
    );
    until(cx, |cx| {
        fixture.facts.read_with(cx, |facts, _| {
            matches!(
                facts.flag(CountryCode::new("xx").unwrap()),
                Some(crate::FlagState::Unavailable)
            )
        })
    });
    let mut sessions = Vec::new();
    let mut gates = Vec::new();
    for _ in 0..32 {
        let gate = Gate::pending();
        fixture.os.held_next(Ok(None), gate.clone());
        let session = fs();
        fixture.request(session.clone(), cx).unwrap();
        sessions.push(session);
        gates.push(gate);
    }
    until(cx, |_| fixture.os.calls() == 32);
    let waiting = fs();
    assert!(matches!(
        fixture.request(waiting.clone(), cx),
        Err(crate::FactsAdmissionError::Capacity {
            needed: 1,
            available: 0
        })
    ));
    let first_flag = Arc::new(nocterm_connections_model::test_support::BlockingGate::default());
    let second_flag = Arc::new(nocterm_connections_model::test_support::BlockingGate::default());
    let _first_release = first_flag.release_on_drop();
    let _second_release = second_flag.release_on_drop();
    fixture.script.suspend_next_flag(first_flag.clone());
    fixture.script.suspend_next_flag(second_flag.clone());
    fixture
        .facts
        .update(cx, |facts, cx| {
            facts.request_flag(CountryCode::new("fr").unwrap(), cx)
        })
        .unwrap();
    fixture
        .facts
        .update(cx, |facts, cx| {
            facts.request_flag(CountryCode::new("gb").unwrap(), cx)
        })
        .unwrap();
    gates[0].release();
    until(cx, |_| first_flag.entered() == 1);
    assert_eq!(fixture.jobs(cx), 0);
    gates[1].release();
    until(cx, |cx| fixture.jobs(cx) == 1);
    assert_eq!(
        second_flag.entered(),
        0,
        "probe lane keeps the released slot"
    );
    let probe = Gate::pending();
    fixture.os.held_next(Ok(None), probe.clone());
    fixture.request(waiting.clone(), cx).unwrap();
    until(cx, |_| fixture.os.calls() == 33);
    first_flag.release();
    until(cx, |_| second_flag.entered() == 1);
    assert_eq!(fixture.jobs(cx), 0);
    for gate in gates {
        gate.release();
    }
    probe.release();
    second_flag.release();
    until(cx, |cx| fixture.jobs(cx) == 32);
    assert_eq!(fixture.os.calls(), 33);
    drop((sessions, waiting));
}

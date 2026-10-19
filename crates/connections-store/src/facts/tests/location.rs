use super::*;
use nocterm_connections_model::test_support::{BlockingGate, ObservationCall};
#[gpui::test]
fn blocking_country_jobs_bind_policy_history_and_keep_physical_capacity(cx: &mut TestAppContext) {
    let native = threaded(cx);
    for unrelated in [false, true] {
        let mut server = profile();
        server.icon = Some("debian".into());
        let fixture = Fixture::new(
            server,
            Default::default(),
            true,
            native.clone(),
            ready_writer(),
            cx,
        );
        let session = fs();
        let gate = Arc::new(BlockingGate::default());
        let _release = gate.release_on_drop();
        fixture.country_answer();
        fixture.script.suspend_next_country(gate.clone());
        fixture.request(session.clone(), cx).unwrap();
        until(cx, |_| gate.entered() == 1);
        let receipts = cx.update(|cx| {
            if unrelated {
                vec![
                    fixture
                        .settings
                        .update::<PolicySettings>(cx, |policy| policy.unrelated += 1),
                ]
            } else {
                vec![
                    fixture
                        .settings
                        .update::<PolicySettings>(cx, |policy| policy.enabled = false),
                    fixture
                        .settings
                        .update::<PolicySettings>(cx, |policy| policy.enabled = true),
                ]
            }
        });
        cx.run_until_parked();
        for receipt in receipts {
            futures::executor::block_on(receipt).unwrap();
        }
        assert_eq!(fixture.jobs(cx), 31);
        gate.release();
        until(cx, |cx| fixture.jobs(cx) == 32);
        assert_eq!(fixture.raw_country(cx), None);
        assert!(
            !fixture
                .script
                .calls()
                .iter()
                .any(|call| matches!(call, ObservationCall::Flag(_)))
        );
        fixture.country_answer();
        fixture.request(session.clone(), cx).unwrap();
        until(cx, |cx| fixture.raw_country(cx).as_deref() == Some("de"));
    }
}
#[gpui::test]
fn country_completion_does_not_renew_through_profile_only_flag_request(cx: &mut TestAppContext) {
    let native = threaded(cx);
    let mut server = profile();
    server.icon = Some("debian".into());
    let fixture = Fixture::new(server, Default::default(), true, native, ready_writer(), cx);
    let session = fs();
    let gate = Arc::new(BlockingGate::default());
    let _release = gate.release_on_drop();
    fixture.country_answer();
    fixture.script.suspend_next_country(gate.clone());
    fixture.request(session.clone(), cx).unwrap();
    until(cx, |_| gate.entered() == 1);
    let mut chosen = fixture.profile.clone();
    chosen.country = Some("fr".into());
    fixture.save(chosen, cx);
    fixture.save(fixture.profile.clone(), cx);
    fixture
        .facts
        .update(cx, |facts, cx| {
            facts.request_flag(CountryCode::new("de").unwrap(), cx)
        })
        .unwrap();
    gate.release();
    until(cx, |cx| fixture.jobs(cx) == 32);
    assert_eq!(fixture.raw_country(cx), None);
}
#[gpui::test]
fn persisted_ip_cache_bypasses_locator_and_invalid_code_is_rechecked(cx: &mut TestAppContext) {
    let native = threaded(cx);
    for (cached, expected) in [("de", 0), ("../", 1)] {
        let known: Known =
            toml::from_str(&format!("[geoip]\n\"8.8.8.8\" = \"{cached}\"\n")).unwrap();
        let mut server = profile();
        server.icon = Some("debian".into());
        let fixture = Fixture::new(server, known, true, native.clone(), ready_writer(), cx);
        let session = fs();
        fixture.country_answer();
        fixture.request(session.clone(), cx).unwrap();
        until(cx, |cx| fixture.raw_country(cx).as_deref() == Some("de"));
        assert_eq!(
            fixture
                .script
                .calls()
                .iter()
                .filter(|call| matches!(call, ObservationCall::Country(_)))
                .count(),
            expected
        );
    }
}
#[gpui::test]
fn offline_installation_does_not_call_observation_ports(cx: &mut TestAppContext) {
    let fixture = Fixture::memory(false, cx);
    let session = fs();
    fixture.request(session.clone(), cx).unwrap();
    fixture
        .facts
        .update(cx, |facts, cx| {
            facts.request_flag(CountryCode::new("de").unwrap(), cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert!(fixture.script.calls().is_empty());
}

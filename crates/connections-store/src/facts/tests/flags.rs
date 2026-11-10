use super::*;
use crate::FlagState;
use nocterm_connections_model::observation::MAX_FLAG_BYTES;
#[gpui::test]
fn independent_flags_need_no_session_or_country_detection_and_negatively_cache(
    cx: &mut TestAppContext,
) {
    let native = threaded(cx);
    let fixture = Fixture::new(
        profile(),
        Default::default(),
        true,
        native,
        ready_writer(),
        cx,
    );
    fixture.publish_policy(|policy| policy.enabled = false, cx);
    let code = CountryCode::new("de").unwrap();
    fixture.script.flag_next(Ok(Some(b"<svg/>".to_vec())));
    fixture
        .facts
        .update(cx, |facts, cx| facts.request_flag(code, cx))
        .unwrap();
    until(cx, |cx| {
        fixture.facts.read_with(cx, |facts, _| {
            matches!(facts.flag(code), Some(FlagState::Ready(_)))
        })
    });
    fixture
        .facts
        .update(cx, |facts, cx| facts.request_flag(code, cx))
        .unwrap();
    assert_eq!(fixture.script.calls().len(), 1);
    let absent = CountryCode::new("xx").unwrap();
    fixture
        .facts
        .update(cx, |facts, cx| facts.request_flag(absent, cx))
        .unwrap();
    until(cx, |cx| {
        fixture.facts.read_with(cx, |facts, _| {
            matches!(facts.flag(absent), Some(FlagState::Unavailable))
        })
    });
    fixture
        .facts
        .update(cx, |facts, cx| facts.request_flag(absent, cx))
        .unwrap();
    assert_eq!(fixture.script.calls().len(), 2);
}
#[gpui::test]
fn oversized_supplied_flag_is_unavailable_at_shared_body_bound(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        profile(),
        Default::default(),
        true,
        threaded(cx),
        ready_writer(),
        cx,
    );
    let code = CountryCode::new("de").unwrap();
    fixture
        .script
        .flag_next(Ok(Some(vec![0; MAX_FLAG_BYTES as usize + 1])));
    fixture
        .facts
        .update(cx, |facts, cx| facts.request_flag(code, cx))
        .unwrap();
    until(cx, |cx| {
        fixture.facts.read_with(cx, |facts, _| {
            matches!(facts.flag(code), Some(FlagState::Unavailable))
        })
    });
}

#[gpui::test]
fn manual_saved_countries_start_without_a_known_row_or_live_session(cx: &mut TestAppContext) {
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
    let code = CountryCode::new("xx").unwrap();
    until(cx, |cx| {
        fixture.facts.read_with(cx, |facts, _| {
            matches!(facts.flag(code), Some(FlagState::Unavailable))
        })
    });
    assert_eq!(
        fixture.script.calls(),
        vec![nocterm_connections_model::test_support::ObservationCall::Flag(code)]
    );
    assert_eq!(fixture.os.calls(), 0);
}

fn transient_flag_failure() -> nocterm_connections_model::observation::ObservationError {
    nocterm_connections_model::observation::ObservationError::Network(Box::new(
        std::io::Error::from(std::io::ErrorKind::NotConnected),
    ))
}

#[gpui::test]
fn transient_flag_failure_retries_after_cooldown_and_publishes_the_new_source(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(
        profile(),
        Default::default(),
        true,
        threaded(cx),
        ready_writer(),
        cx,
    );
    let code = CountryCode::new("de").unwrap();
    fixture.script.flag_next(Err(transient_flag_failure()));
    fixture.script.flag_next(Ok(Some(b"<svg/>".to_vec())));
    fixture
        .facts
        .update(cx, |facts, cx| facts.request_flag(code, cx))
        .unwrap();
    until(cx, |cx| {
        fixture.facts.read_with(cx, |facts, _| {
            matches!(facts.flag(code), Some(FlagState::Unavailable))
        })
    });
    for _ in 0..20 {
        fixture
            .facts
            .update(cx, |facts, cx| facts.request_flag(code, cx))
            .unwrap();
    }
    assert_eq!(fixture.script.calls().len(), 1);
    cx.executor().advance_clock(Duration::from_secs(5));
    until(cx, |cx| {
        fixture.facts.read_with(cx, |facts, _| {
            matches!(facts.flag(code), Some(FlagState::Ready(_)))
        })
    });
    assert_eq!(fixture.script.calls().len(), 2);
}

#[gpui::test]
fn an_offline_flag_has_at_most_three_automatic_attempts(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        profile(),
        Default::default(),
        true,
        threaded(cx),
        ready_writer(),
        cx,
    );
    let code = CountryCode::new("de").unwrap();
    for _ in 0..3 {
        fixture.script.flag_next(Err(transient_flag_failure()));
    }
    fixture
        .facts
        .update(cx, |facts, cx| facts.request_flag(code, cx))
        .unwrap();
    for attempt in 1..=3 {
        until(cx, |cx| {
            fixture.facts.read_with(cx, |facts, _| {
                matches!(facts.flag(code), Some(FlagState::Unavailable))
            }) && fixture.script.calls().len() == attempt
        });
        cx.executor().advance_clock(Duration::from_secs(31));
        cx.run_until_parked();
    }
    assert_eq!(fixture.script.calls().len(), 3);
    cx.executor().advance_clock(Duration::from_secs(300));
    fixture
        .facts
        .update(cx, |facts, cx| facts.request_flag(code, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(fixture.script.calls().len(), 3);
}

#[gpui::test]
fn a_missing_flag_is_permanently_negative_without_network_retries(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        profile(),
        Default::default(),
        true,
        threaded(cx),
        ready_writer(),
        cx,
    );
    let code = CountryCode::new("xx").unwrap();
    fixture.script.flag_next(Err(
        nocterm_connections_model::observation::ObservationError::Status {
            status: 404,
            source: Box::new(std::io::Error::from(std::io::ErrorKind::NotFound)),
        },
    ));
    fixture
        .facts
        .update(cx, |facts, cx| facts.request_flag(code, cx))
        .unwrap();
    until(cx, |cx| {
        fixture.facts.read_with(cx, |facts, _| {
            matches!(facts.flag(code), Some(FlagState::Unavailable))
        })
    });
    cx.executor().advance_clock(Duration::from_secs(300));
    fixture
        .facts
        .update(cx, |facts, cx| facts.request_flag(code, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(fixture.script.calls().len(), 1);
}

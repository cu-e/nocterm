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

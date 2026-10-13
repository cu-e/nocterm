use super::*;
use nocterm_connections_model::{
    observation::{ObservationError, PublicAddress},
    test_support::{ObservationCall, ScriptedObservations},
};
use nocterm_session::testing::InMemoryFs;

fn deps(script: &Arc<ScriptedObservations>, online: bool) -> ObservationDeps {
    ObservationDeps::new(
        script.clone(),
        script.clone(),
        script.clone(),
        online,
        Arc::new(nocterm_connections_model::os_observation::OfflineOsDetector),
    )
}
fn profile() -> Profile {
    Profile {
        id: nocterm_connections_model::test_support::profile_id_for(
            "crates/connections/src/facts/observation_tests.rs:1",
        ),
        name: "web".into(),
        description: String::new(),
        target: nocterm_session::Target::new("root", "example.com", 22),
        auth: Default::default(),
        options: Default::default(),
        group: None,
        credential: None,
        launch: None,
        icon: Some("debian".into()),
        icon_color: None,
        country: None,
    }
}
fn answer(script: &ScriptedObservations) {
    script.resolve_next(Ok(PublicAddress::new("8.8.8.8".parse().unwrap())));
    script.country_next(Ok(Some(CountryCode::new("de").unwrap())));
}

#[gpui_kit::test]
async fn injected_country_and_flag_are_published_and_memory_flag_is_reused(
    cx: &mut gpui_kit::TestAppContext,
) {
    let script = Arc::new(ScriptedObservations::default());
    answer(&script);
    script.flag_next(Ok(Some(b"<svg/>".to_vec())));
    let facts = cx.new(|_| ServerFacts::in_memory(deps(&script, true)));
    let first = profile();
    facts.update(cx, |facts, cx| {
        facts.probe(
            &first,
            "example.com",
            Arc::new(InMemoryFs::default()),
            true,
            cx,
        )
    });
    cx.run_until_parked();
    assert_eq!(
        facts.read_with(cx, |facts, cx| facts
            .country(first.id, cx)
            .map(str::to_owned)),
        Some("de".into())
    );
    let image = facts.read_with(cx, |facts, _| facts.flag("de")).unwrap();
    answer(&script);
    let second = profile();
    facts.update(cx, |facts, cx| {
        facts.probe(
            &second,
            "other.example",
            Arc::new(InMemoryFs::default()),
            true,
            cx,
        )
    });
    cx.run_until_parked();
    let repeated = facts.read_with(cx, |facts, _| facts.flag("de")).unwrap();
    assert!(Arc::ptr_eq(&image, &repeated));
    assert_eq!(
        script
            .calls()
            .iter()
            .filter(|call| matches!(call, ObservationCall::Flag(_)))
            .count(),
        1
    );
    let mut chosen = first.clone();
    chosen.country = Some("fr".into());
    assert_eq!(
        facts.read_with(cx, |facts, cx| facts
            .shown_country(&chosen, true, cx)
            .map(str::to_owned)),
        Some("fr".into())
    );
}

#[gpui_kit::test]
async fn downloaded_flag_failure_uses_embedded_fallback(cx: &mut gpui_kit::TestAppContext) {
    let script = Arc::new(ScriptedObservations::default());
    answer(&script);
    script.flag_next(Err(ObservationError::BodyLimit { limit: 256 * 1024 }));
    let facts = cx.new(|_| ServerFacts::in_memory(deps(&script, true)));
    let profile = profile();
    facts.update(cx, |facts, cx| {
        facts.probe(
            &profile,
            "example.com",
            Arc::new(InMemoryFs::default()),
            true,
            cx,
        )
    });
    cx.run_until_parked();
    assert!(facts.read_with(cx, |facts, _| facts.flag("de").is_some()));
}

#[gpui_kit::test]
async fn offline_installation_does_not_call_observation_ports(cx: &mut gpui_kit::TestAppContext) {
    let script = Arc::new(ScriptedObservations::default());
    let facts = cx.new(|_| ServerFacts::in_memory(deps(&script, false)));
    facts.update(cx, |facts, cx| {
        facts.probe(
            &profile(),
            "example.com",
            Arc::new(InMemoryFs::default()),
            true,
            cx,
        )
    });
    cx.run_until_parked();
    assert!(script.calls().is_empty());
}

#[gpui_kit::test]
async fn persisted_ip_cache_bypasses_locator_and_invalid_code_is_rechecked(
    cx: &mut gpui_kit::TestAppContext,
) {
    for (cached, calls) in [("de", 0), ("../", 1)] {
        let known: Known =
            toml::from_str(&format!("[geoip]\n\"8.8.8.8\" = \"{cached}\"\n")).unwrap();
        let script = Arc::new(ScriptedObservations::default());
        answer(&script);
        let facts = cx.new(|_| {
            ServerFacts::with_backend(
                known,
                Arc::new(|_| Box::pin(async { Ok(()) })),
                deps(&script, true),
            )
        });
        let profile = profile();
        facts.update(cx, |facts, cx| {
            facts.probe(
                &profile,
                "example.com",
                Arc::new(InMemoryFs::default()),
                true,
                cx,
            )
        });
        cx.run_until_parked();
        assert_eq!(
            facts.read_with(cx, |facts, cx| facts
                .country(profile.id, cx)
                .map(str::to_owned)),
            Some("de".into())
        );
        assert_eq!(
            script
                .calls()
                .iter()
                .filter(|call| matches!(call, ObservationCall::Country(_)))
                .count(),
            calls
        );
    }
}

use super::*;
use nocterm_connections_model::{
    observation::{CountryCode, ObservationError, PublicAddress},
    test_support::ObservationCall,
};
fn answer(script: &ScriptedObservations) {
    script.resolve_next(Ok(PublicAddress::new("8.8.8.8".parse().unwrap())));
    script.country_next(Ok(Some(CountryCode::new("de").unwrap())));
}
#[gpui_kit::test]
fn injected_country_and_flag_are_published_and_memory_flag_is_reused(cx: &mut TestAppContext) {
    let mut scene = Scene::new(true, cx);
    answer(&scene.script);
    scene.script.flag_next(Ok(Some(b"<svg/>".to_vec())));
    scene.add(
        scene.window,
        &scene.workspace,
        Arc::new(InMemoryFs::default()),
        cx,
    );
    scene.scan(cx);
    until(cx, |cx| {
        scene.installed.facts().read_with(cx, |facts, cx| {
            facts.country(scene.profile.id, cx) == Some("de")
        })
    });
    until(cx, |cx| {
        cx.read(|cx| scene.installed.flag("de", cx).is_some())
    });
    let first = cx.read(|cx| scene.installed.flag("de", cx).unwrap());
    // A real committed endpoint replacement re-runs country resolution while reusing raw flags.
    scene.profile.target.host = "other.example.com".into();
    let receipt = scene.installed.connections().update(cx, |connections, cx| {
        connections.save_profile(scene.profile.clone(), cx)
    });
    cx.run_until_parked();
    futures::executor::block_on(receipt).unwrap();
    answer(&scene.script);
    scene.add(
        scene.window,
        &scene.workspace,
        Arc::new(InMemoryFs::default()),
        cx,
    );
    scene.scan(cx);
    until(cx, |cx| {
        scene.installed.facts().read_with(cx, |facts, cx| {
            facts.country(scene.profile.id, cx) == Some("de")
        })
    });
    let repeated = cx.read(|cx| scene.installed.flag("de", cx).unwrap());
    assert!(Arc::ptr_eq(&first, &repeated));
    assert_eq!(
        scene
            .script
            .calls()
            .iter()
            .filter(|call| matches!(call, ObservationCall::Flag(_)))
            .count(),
        1
    );
    let mut chosen = scene.profile.clone();
    chosen.country = Some("fr".into());
    assert_eq!(
        cx.read(|cx| scene.installed.shown_country(&chosen, cx)),
        Some("fr".into())
    );
}
#[gpui_kit::test]
fn downloaded_flag_failure_uses_embedded_fallback(cx: &mut TestAppContext) {
    let scene = Scene::new(true, cx);
    answer(&scene.script);
    scene
        .script
        .flag_next(Err(ObservationError::BodyLimit { limit: 256 * 1024 }));
    scene.add(
        scene.window,
        &scene.workspace,
        Arc::new(InMemoryFs::default()),
        cx,
    );
    scene.scan(cx);
    until(cx, |cx| {
        cx.read(|cx| scene.installed.flag("de", cx).is_some())
    });
    assert_eq!(
        scene
            .script
            .calls()
            .iter()
            .filter(|call| matches!(call, ObservationCall::Flag(_)))
            .count(),
        1
    );
}
#[gpui_kit::test]
fn startup_known_country_keys_are_loaded_without_any_remaining_profile_or_session(
    cx: &mut TestAppContext,
) {
    let script = Arc::new(ScriptedObservations::default());
    script.flag_next(Ok(Some(b"<svg/>".to_vec())));
    let orphan = nocterm_connections_model::test_support::profile_id_for("orphan");
    let known = toml::from_str(&format!("[server.\"{orphan}\"]\ncountry = \"xx\"\n")).unwrap();
    let scene = Scene::with_known(true, known, script.clone(), cx);
    until(cx, |cx| {
        cx.read(|cx| scene.installed.flag("xx", cx).is_some())
    });
    assert_eq!(scene.os.calls(), 0);
    assert_eq!(
        script.calls(),
        vec![ObservationCall::Flag(CountryCode::new("xx").unwrap())]
    );
    assert!(
        scene
            .installed
            .facts()
            .read_with(cx, |facts, cx| facts.country(orphan, cx).is_none())
    );
}
#[gpui_kit::test]
fn manual_and_unsaved_country_requests_work_when_detection_is_disabled_without_a_session(
    cx: &mut TestAppContext,
) {
    let scene = Scene::new(true, cx);
    let receipt = cx.update(|cx| {
        scene
            .installed
            .settings()
            .update::<nocterm_ui::AppearanceSettings>(cx, |settings| {
                settings.detect_server_country = false
            })
    });
    cx.run_until_parked();
    futures::executor::block_on(receipt).unwrap();
    let mut profile = scene.profile.clone();
    profile.country = Some("de".into());
    scene.script.flag_next(Ok(Some(b"<svg/>".to_vec())));
    let receipt = scene.installed.connections().update(cx, |connections, cx| {
        connections.save_profile(profile.clone(), cx)
    });
    cx.run_until_parked();
    futures::executor::block_on(receipt).unwrap();
    until(cx, |cx| {
        cx.read(|cx| scene.installed.flag("de", cx).is_some())
    });
    assert_eq!(
        cx.read(|cx| scene.installed.shown_country(&profile, cx)),
        Some("de".into())
    );
    scene.script.flag_next(Ok(Some(b"<svg/>".to_vec())));
    cx.update(|cx| {
        scene.installed.facts().update(cx, |facts, cx| {
            facts
                .request_flag(CountryCode::new("fr").unwrap(), cx)
                .unwrap()
        })
    });
    until(cx, |cx| {
        cx.read(|cx| scene.installed.flag("fr", cx).is_some())
    });
    assert_eq!(scene.os.calls(), 0);
    assert!(
        scene
            .script
            .calls()
            .iter()
            .all(|call| matches!(call, ObservationCall::Flag(_)))
    );
    assert_eq!(scene.script.calls().len(), 2);
}

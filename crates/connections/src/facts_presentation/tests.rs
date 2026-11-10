use super::*;
use nocterm_connections_model::observation::FlagBytes;
#[test]
fn ready_image_projection_reuses_its_single_shared_allocation() {
    let cache = FactsPresentation::default();
    let code = CountryCode::new("de").unwrap();
    let ready = FlagState::Ready(FlagBytes::new(b"<svg/>".to_vec()).unwrap());
    let first = cache.flag(code, Some(ready.clone())).unwrap();
    let repeated = cache.flag(code, Some(ready)).unwrap();
    assert!(Arc::ptr_eq(&first, &repeated));
}
#[test]
fn embedded_fallback_waits_until_raw_negative_result() {
    let cache = FactsPresentation::default();
    let code = CountryCode::new("de").unwrap();
    assert!(cache.flag(code, Some(FlagState::Requested)).is_none());
    assert!(cache.flag(code, Some(FlagState::Loading)).is_none());
    assert!(cache.flag(code, Some(FlagState::Unavailable)).is_some());
    assert!(
        cache
            .flag(
                CountryCode::new("xx").unwrap(),
                Some(FlagState::Unavailable)
            )
            .is_none()
    );
}

#[test]
fn ready_source_changes_and_retries_replace_the_presented_flag() {
    let cache = FactsPresentation::default();
    let code = CountryCode::new("de").unwrap();
    let ready = || FlagState::Ready(FlagBytes::new(b"<svg/>".to_vec()).unwrap());
    let old = cache.flag(code, Some(ready())).unwrap();
    let changed = cache.flag(code, Some(ready())).unwrap();
    assert!(
        !Arc::ptr_eq(&old, &changed),
        "replacement raw source refreshes presentation"
    );
    assert!(cache.flag(code, Some(FlagState::Loading)).is_none());
    let fallback = cache.flag(code, Some(FlagState::Unavailable)).unwrap();
    assert!(!Arc::ptr_eq(&changed, &fallback));
    let current = cache.flag(code, Some(ready())).unwrap();
    assert!(!Arc::ptr_eq(&fallback, &current));
}

#[test]
fn projected_flags_evict_old_countries_at_the_byte_budget() {
    let cache = FactsPresentation::default();
    let first_code = CountryCode::new("aa").unwrap();
    let raw = FlagBytes::new(vec![b' '; 256 * 1024]).unwrap();
    let first = cache
        .flag(first_code, Some(FlagState::Ready(raw.clone())))
        .unwrap();
    for second in b'b'..=b'j' {
        let code = CountryCode::new(&format!("a{}", char::from(second))).unwrap();
        cache.flag(code, Some(FlagState::Ready(raw.clone())));
    }
    let revisited = cache.flag(first_code, Some(FlagState::Ready(raw))).unwrap();
    assert!(!Arc::ptr_eq(&first, &revisited));
}

#[test]
fn projected_icons_share_canonical_color_and_evict_old_choices() {
    let cache = FactsPresentation::default();
    let mut profile = nocterm_connections_model::profiles::Profile {
        id: nocterm_connections_model::test_support::profile_id_for("icons"),
        name: "icons".into(),
        target: nocterm_session::Target::new("ci", "icons", 22),
        description: String::new(),
        options: Default::default(),
        auth: Default::default(),
        group: None,
        credential: None,
        launch: None,
        icon: Some("ubuntu".into()),
        icon_color: Some("#abc".into()),
        country: None,
        jump_hosts: Vec::new(),
    };
    let first = cache.icon(&profile, None).unwrap();
    profile.icon_color = Some("#AABBCC".into());
    assert!(Arc::ptr_eq(&first, &cache.icon(&profile, None).unwrap()));
    for color in 0..256 {
        profile.icon_color = Some(format!("#{color:06x}"));
        cache.icon(&profile, None);
    }
    profile.icon_color = Some("#abc".into());
    assert!(!Arc::ptr_eq(&first, &cache.icon(&profile, None).unwrap()));
    profile.icon_color = Some("invalid".into());
    let default = cache.icon(&profile, None).unwrap();
    profile.icon_color = None;
    assert!(Arc::ptr_eq(&default, &cache.icon(&profile, None).unwrap()));
}

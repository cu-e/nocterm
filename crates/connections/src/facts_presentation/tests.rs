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

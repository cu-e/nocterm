use alloc::{string::ToString, vec::Vec};

use crate::{
    Effect, EffectRequest, Effects, EffectsError, Event, HostError, LimitError, MAX_EFFECTS,
    capability::{
        notifications::{NoticeTone, Post},
        storage::{MAX_VALUE, Set},
    },
};

fn store(key: &str) -> Effect {
    Effect::StorageSet(Set {
        key: key.to_string(),
        value: "v".to_string(),
    })
}

fn request(key: Option<&str>, effect: Effect) -> EffectRequest {
    EffectRequest {
        key: key.map(ToString::to_string),
        effect,
    }
}

#[test]
fn a_call_returns_at_most_eight_effects() {
    let many: Vec<_> = (0..=MAX_EFFECTS)
        .map(|i| request(None, store(&alloc::format!("k{i}"))))
        .collect();
    assert_eq!(Effects(many[..MAX_EFFECTS].to_vec()).check(), Ok(()));
    assert_eq!(Effects(many).check(), Err(EffectsError::TooMany));
}

#[test]
fn correlation_keys_are_unique_and_plain() {
    let twice = Effects(Vec::from([
        request(Some("a"), store("one")),
        request(Some("a"), store("two")),
    ]));
    assert_eq!(
        twice.check(),
        Err(EffectsError::DuplicateKey { key: "a".into() })
    );
    let bad = Effects(Vec::from([request(Some("A b"), store("one"))]));
    assert!(matches!(
        bad.check(),
        Err(EffectsError::Limit { index: 0, .. })
    ));
}

#[test]
fn each_effect_is_checked_against_its_own_limits() {
    let long = Effect::StorageSet(Set {
        key: "k".into(),
        value: "x".repeat(MAX_VALUE + 1),
    });
    assert!(matches!(
        Effects(Vec::from([request(None, long)])).check(),
        Err(EffectsError::Limit {
            index: 0,
            source: LimitError::TooLong { .. }
        })
    ));
    let hostile = Effect::NotificationsPost(Post {
        tone: NoticeTone::Caution,
        text: "pay\u{202E}now".into(),
        notice_key: None,
    });
    assert!(matches!(
        Effects(Vec::from([request(None, hostile)])).check(),
        Err(EffectsError::Limit {
            source: LimitError::Unsanitized { .. },
            ..
        })
    ));
}

#[test]
fn events_carry_no_key_or_character_data() {
    let event: Event = serde_json::from_str(r#"{"activated":{"key":"go"}}"#).unwrap();
    assert_eq!(event, Event::Activated { key: "go".into() });
    assert!(serde_json::from_str::<Event>(r#"{"key-pressed":{"code":"a"}}"#).is_err());
}

#[test]
fn host_errors_use_frozen_names_and_never_include_faulted() {
    let json = serde_json::to_string(&HostError::RateLimited { retry_after_ms: 5 }).unwrap();
    assert_eq!(
        json,
        r#"{"rate-limited":{"retry-after-ms":5}}"#.replace("retry-after-ms", "retry_after_ms")
    );
    assert_eq!(
        serde_json::to_string(&HostError::NotGranted).unwrap(),
        "\"not-granted\""
    );
    assert!(serde_json::from_str::<HostError>("\"faulted\"").is_err());
}

use super::*;
use format::encode_records;
fn binding() -> CredentialBinding {
    CredentialBinding::Password {
        target: nocterm_session::Target::new("test", "host", 22),
    }
}
#[test]
fn bounded_payload_parser_preserves_secret_bytes_and_rejects_lengths() {
    let secret = "a\"quoted\\password\nwith unicode λ";
    let records = vec![Record {
        info: CredentialInfo {
            id: CredentialId::from_bytes([42; 16]),
            label: "label".into(),
            binding: binding(),
        },
        secret: Secret::new(secret),
    }];
    let encoded = encode_records(&records).unwrap();
    let decoded = decode_records(&encoded).unwrap();
    assert_eq!(decoded[0].secret.expose(), secret);
    for length in [0, 3, 7, encoded.len() - 1] {
        assert!(decode_records(&encoded[..length]).is_err());
    }
    let mut hostile = encoded.to_vec();
    hostile[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(decode_records(&hostile).is_err());
    hostile[..8].copy_from_slice(b"CRD1\xff\xff\xff\xff");
    assert!(decode_records(&hostile).is_err());
}

fn material() -> CreateMaterial {
    CreateMaterial::new(
        [1; 16],
        VaultKey::new([2; 32]),
        [3; 16],
        SealNonces::new([4; 24], [5; 24]),
    )
}
fn state() -> VaultState {
    let mut state = VaultState::locked();
    let p = state
        .prepare_create(Secret::new("long master password"), material())
        .unwrap();
    state.accept_write(p);
    state
}
#[test]
fn prepared_transition_leaves_state_until_accepted() {
    let mut state = state();
    let p = state
        .prepare_put(
            CredentialId::from_bytes([1; 16]),
            "account".into(),
            binding(),
            Secret::new("secret"),
            SealNonces::new([6; 24], [7; 24]),
        )
        .unwrap();
    assert!(state.list().unwrap().is_empty());
    state.accept_write(p);
    assert_eq!(state.list().unwrap().len(), 1);
    assert!(matches!(
        state.get(
            CredentialId::from_bytes([1; 16]),
            &CredentialBinding::Password {
                target: nocterm_session::Target::new("other", "host", 22)
            }
        ),
        Err(VaultError::Binding)
    ));
}
#[test]
fn discarded_candidate_preserves_last_state() {
    let state = state();
    let old = state.revision().unwrap().to_vec();
    let p = state
        .prepare_delete(
            CredentialId::from_bytes([1; 16]),
            SealNonces::new([8; 24], [9; 24]),
        )
        .unwrap();
    drop(p);
    assert_eq!(state.revision().unwrap(), old);
}
#[test]
fn hostile_headers_reject_before_derivation() {
    let state = state();
    let original = state.revision().unwrap();
    for n in [0, 7, 107, 155, original.len() - 1] {
        assert!(EnvelopeInfo::parse(&original[..n]).is_err());
    }
    for offset in [8, 9, 10, 11, 12, 16, 20, 104] {
        let mut bad = original.to_vec();
        bad[offset] ^= 1;
        assert!(matches!(EnvelopeInfo::parse(&bad), Err(VaultError::Format)));
    }
}

#[test]
fn authenticated_candidates_roundtrip_unicode_and_reject_tampered_payloads() {
    let mut state = state();
    let id = CredentialId::from_bytes([9; 16]);
    let secret = "quoted \\\" value\nλ secret";
    let put = state
        .prepare_put(
            id,
            "private label".into(),
            binding(),
            Secret::new(secret),
            SealNonces::new([12; 24], [13; 24]),
        )
        .unwrap();
    state.accept_write(put);
    let encrypted = state.revision().unwrap().to_vec();
    assert!(
        !encrypted
            .windows(secret.len())
            .any(|part| part == secret.as_bytes())
    );
    let key = state.device_enrollment().unwrap().take_key();
    assert_eq!(format!("{key:?}"), "VaultKey([REDACTED])");
    let mut corrupt = encrypted.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(matches!(
        VaultState::prepare_device_unlock(corrupt, key.clone()),
        Err(VaultError::Authentication)
    ));
    state.lock();
    assert!(matches!(state.get(id, &binding()), Err(VaultError::Locked)));
    let unlock = VaultState::prepare_device_unlock(encrypted, key).unwrap();
    assert!(!state.is_unlocked());
    state.accept_unlock(unlock);
    assert_eq!(state.get(id, &binding()).unwrap().expose(), secret);
}

#[test]
fn identity_updates_and_rejected_bindings_preserve_accepted_records() {
    let mut state = state();
    let id = CredentialId::from_bytes([10; 16]);
    let first = state
        .prepare_put(
            id,
            "first".into(),
            binding(),
            Secret::new("first secret"),
            SealNonces::new([14; 24], [15; 24]),
        )
        .unwrap();
    state.accept_write(first);
    let updated = state
        .prepare_put(
            id,
            "updated".into(),
            binding(),
            Secret::new("new secret"),
            SealNonces::new([16; 24], [17; 24]),
        )
        .unwrap();
    state.accept_write(updated);
    assert_eq!(state.list().unwrap().len(), 1);
    assert_eq!(state.get(id, &binding()).unwrap().expose(), "new secret");
    let revision = state.revision().unwrap().to_vec();
    let different = CredentialBinding::Password {
        target: nocterm_session::Target::new("other", "host", 22),
    };
    assert!(matches!(
        state.prepare_put(
            id,
            "reject".into(),
            different,
            Secret::new("secret"),
            SealNonces::new([18; 24], [19; 24])
        ),
        Err(VaultError::Binding)
    ));
    assert!(matches!(
        state.prepare_put(
            id,
            "x".repeat(1025),
            binding(),
            Secret::new("secret"),
            SealNonces::new([20; 24], [21; 24])
        ),
        Err(VaultError::Limit)
    ));
    assert_eq!(state.revision().unwrap(), revision);
    assert_eq!(state.get(id, &binding()).unwrap().expose(), "new secret");
}

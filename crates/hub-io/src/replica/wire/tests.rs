use nocterm_connections_model::test_support::profile_id_for;
use nocterm_keys::OpenError;

use super::*;

fn profile(name: &str) -> Profile {
    serde_json::from_value(serde_json::json!({
        "id": profile_id_for(name),
        "name": name,
        "host": "example.org",
        "user": "ann",
    }))
    .unwrap()
}

fn key() -> CollectionKey {
    CollectionKey::generate().unwrap()
}

fn object(
    key: &CollectionKey,
    profile_id: ProfileId,
    kind: &str,
    plain: &[u8],
) -> v1::StoredObject {
    let object_id = profile_id.to_string();
    v1::StoredObject {
        collection_id: "c".into(),
        object_id: object_id.clone(),
        version: 3,
        kind: kind.into(),
        ciphertext: key.seal_object("c", &object_id, kind, plain).unwrap(),
        deleted: false,
        updated_at_ms: 0,
        updated_by_device: String::new(),
        stream_version: 7,
    }
}

#[test]
fn a_profile_written_by_this_client_reads_back_with_its_version() {
    let key = key();
    let web = profile("web");
    let stored = object(&key, web.id, KIND, &encode(&web).unwrap());
    let read = read(ReplicaScope::Personal, "c", &key, &stored)
        .unwrap()
        .unwrap();
    assert_eq!(read.id, web.id);
    assert_eq!(read.version, ReplicaVersion::new(3));
    assert_eq!(read.scope, ReplicaScope::Personal);
    assert_eq!(read.body, RemoteBody::Present(Box::new(web)));
}

#[test]
fn a_tombstone_is_a_deletion_without_opening_anything() {
    let key = key();
    let id = profile_id_for("gone");
    let mut stored = object(&key, id, KIND, b"{}");
    stored.deleted = true;
    stored.ciphertext.clear();
    let read = read(ReplicaScope::Personal, "c", &key, &stored)
        .unwrap()
        .unwrap();
    assert_eq!(read.body, RemoteBody::Deleted);
}

#[test]
fn an_object_this_client_cannot_decode_is_unrecognized() {
    let key = key();
    let web = profile("web");
    let newer = object(&key, web.id, KIND, br#"{"v":2,"profile":{}}"#);
    assert_eq!(
        read(ReplicaScope::Personal, "c", &key, &newer)
            .unwrap()
            .unwrap()
            .body,
        RemoteBody::Unrecognized
    );
    let other_kind = object(&key, web.id, "profile-v2", &encode(&web).unwrap());
    assert_eq!(
        read(ReplicaScope::Personal, "c", &key, &other_kind)
            .unwrap()
            .unwrap()
            .body,
        RemoteBody::Unrecognized
    );
    let garbage = object(&key, web.id, KIND, b"not json");
    assert_eq!(
        read(ReplicaScope::Personal, "c", &key, &garbage)
            .unwrap()
            .unwrap()
            .body,
        RemoteBody::Unrecognized
    );
}

#[test]
fn a_profile_stored_under_another_profiles_id_is_unrecognized() {
    let key = key();
    let web = profile("web");
    let other = profile("other");
    let moved = object(&key, other.id, KIND, &encode(&web).unwrap());
    assert_eq!(
        read(ReplicaScope::Personal, "c", &key, &moved)
            .unwrap()
            .unwrap()
            .body,
        RemoteBody::Unrecognized
    );
}

#[test]
fn an_object_that_does_not_authenticate_is_an_error() {
    let key = key();
    let web = profile("web");
    let mut forged = object(&key, web.id, KIND, &encode(&web).unwrap());
    forged.ciphertext[30] ^= 0xff;
    let error = read(ReplicaScope::Personal, "c", &key, &forged).unwrap_err();
    let ReplicaError::Other(cause) = error else {
        panic!("a forgery is a failure of the replica");
    };
    assert!(cause.downcast_ref::<OpenError>().is_some());
    // The same bytes under another key do not open either.
    let stranger = self::key();
    let sealed = object(&key, web.id, KIND, &encode(&web).unwrap());
    assert!(read(ReplicaScope::Personal, "c", &stranger, &sealed).is_err());
}

#[test]
fn an_object_that_is_not_a_profile_id_is_skipped() {
    let key = key();
    let stored = v1::StoredObject {
        object_id: "not-a-profile".into(),
        ..object(&key, profile_id_for("x"), KIND, b"{}")
    };
    assert!(
        read(ReplicaScope::Personal, "c", &key, &stored)
            .unwrap()
            .is_none()
    );
}

#[test]
fn positions_round_trip_and_an_unreadable_cursor_starts_over() {
    let mut positions = Positions::default();
    positions.set("a", 4);
    let cursor = positions.cursor();
    assert_eq!(Positions::parse(Some(&cursor)).get("a"), 4);
    assert_eq!(
        Positions::parse(Some(&ReplicaCursor::new("???"))).get("a"),
        0
    );
    assert_eq!(Positions::parse(None).get("a"), 0);
}

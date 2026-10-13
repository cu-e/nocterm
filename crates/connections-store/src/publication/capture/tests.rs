//! Captured outcomes preserve transition history through the shared public driver API.
use super::*;
use gpui::AppContext as _;
use nocterm_connections_model::profiles::{Profile, ProfileExpectation};
use nocterm_session::testing::Gate;
use nocterm_store::{Publication, SnapshotWriter};
use std::sync::Mutex;

fn profile() -> Profile {
    Profile {
        id: ProfileId::from_bytes([1; 16]),
        name: "Captured".into(),
        description: String::new(),
        options: Default::default(),
        target: Target::new("ci", "host", 22),
        auth: Auth::Password,
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        group: None,
    }
}
fn documents(profile: &Profile) -> Profiles {
    let mut profiles = Profiles::default();
    profiles.upsert(profile.clone());
    profiles
}
fn save(profile: Profile) -> ProfileMutation {
    ProfileMutation::Save {
        profile: Box::new(profile),
        expected: ProfileExpectation::Any,
    }
}
fn admit(
    entity: &Entity<Persisted<ProfileSnapshot>>,
    mutation: ProfileMutation,
    cx: &mut gpui::TestAppContext,
) -> nocterm_store::Receipt {
    entity
        .update(cx, |state, cx| {
            state.update_checked(
                move |snapshot| snapshot.apply(&mutation, 0, None).map_err(StoreError::edit),
                cx,
            )
        })
        .unwrap()
}

#[gpui::test]
async fn coalesced_equal_payload_capture_keeps_endpoint_and_override_cycles(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile();
    let writes = Arc::new(Mutex::new(Vec::new()));
    let recorded = writes.clone();
    let writer: SnapshotWriter<ProfileSnapshot> = Arc::new(move |snapshot| {
        recorded.lock().unwrap().push(snapshot.into_profiles());
        Box::pin(async { Ok(()) })
    });
    let entity = cx.new(|_| {
        Persisted::new(
            ProfileSnapshot::new(documents(&original)),
            writer,
            Publication::Durable,
            32,
        )
    });
    let mut changed = original.clone();
    changed.target.user = "different".into();
    changed.target.port = 2222;
    changed.icon = Some("debian".into());
    changed.country = Some("de".into());
    let changed = admit(&entity, save(changed), cx);
    let restored = admit(&entity, save(original.clone()), cx);
    let captured = entity.update(cx, |state, _| state.capture());
    let final_projection = captured.final_snapshot().publication();
    assert_eq!(final_projection.generation(), 2);
    assert_eq!(
        final_projection.get(original.id).unwrap().stamp,
        crate::ProfileStamp {
            incarnation: 0,
            endpoint: 2,
            icon_override: 2,
            country_override: 2
        }
    );
    drop(entity);
    let outcome = cx.background_executor.spawn(captured.run()).await;
    assert!(outcome.result().is_ok());
    changed.wait().await.unwrap();
    restored.wait().await.unwrap();
    assert_eq!(
        outcome.committed().snapshot().publication(),
        final_projection
    );
    assert_eq!(*writes.lock().unwrap(), [documents(&original)]);
}

#[gpui::test]
async fn captured_readdition_authority_requires_its_covering_physical_success(
    cx: &mut gpui::TestAppContext,
) {
    for fail_final in [true, false] {
        let original = profile();
        let gate = Gate::pending();
        let writer_gate = gate.clone();
        let first = Arc::new(Mutex::new(true));
        let writer: SnapshotWriter<ProfileSnapshot> = Arc::new(move |_| {
            let active = std::mem::take(&mut *first.lock().unwrap());
            let gate = writer_gate.clone();
            Box::pin(async move {
                if active {
                    gate.wait().await;
                } else if fail_final {
                    return Err(StoreError::PendingWrites);
                }
                Ok(())
            })
        });
        let entity = cx.new(|_| {
            Persisted::new(
                ProfileSnapshot::new(documents(&original)),
                writer,
                Publication::Durable,
                32,
            )
        });
        let mut prefix = original.clone();
        prefix.name = "Prefix".into();
        let active = admit(&entity, save(prefix), cx);
        cx.run_until_parked();
        let deleted = admit(&entity, ProfileMutation::Delete(original.id), cx);
        let readded = admit(&entity, save(original.clone()), cx);
        let captured = entity.update(cx, |state, _| state.capture());
        let final_projection = captured.final_snapshot().publication();
        assert_eq!(
            final_projection.get(original.id).unwrap().stamp.incarnation,
            3
        );
        let drain = cx.background_executor.spawn(captured.run());
        drop(entity);
        gate.release();
        let outcome = drain.await;
        active.wait().await.unwrap();
        assert_eq!(outcome.result().is_ok(), !fail_final);
        assert_eq!(deleted.wait().await.is_ok(), !fail_final);
        assert_eq!(readded.wait().await.is_ok(), !fail_final);
        let committed = outcome.committed().snapshot().publication();
        if fail_final {
            assert_eq!(committed.generation(), 1);
            assert_eq!(committed.get(original.id).unwrap().stamp.incarnation, 0);
        } else {
            assert_eq!(committed, final_projection);
        }
    }
}

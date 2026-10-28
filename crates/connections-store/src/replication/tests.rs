use super::*;
use gpui::AppContext as _;
use nocterm_connections_model::{
    profiles::{Profile, Profiles, Recents},
    replica::{RemoteBody, ReplicaCursor, ReplicaVersion, SyncedEntry},
    test_support::{MemoryReplica, ReplicaCall, profile_id_for},
};
use nocterm_session::{Auth, Target};
use nocterm_store::test_support::{WriteLog, gated_writer};
use std::sync::Mutex;

mod conflicts;
mod lifecycle;
mod spaces;

fn profile(name: &str) -> Profile {
    Profile {
        id: profile_id_for(name),
        name: name.into(),
        description: String::new(),
        options: Default::default(),
        target: Target::new("ci", format!("{name}.example"), 22),
        auth: Auth::Password,
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        group: None,
        jump_hosts: Vec::new(),
    }
}

fn at_host(profile: &Profile, host: &str) -> Profile {
    let mut edited = profile.clone();
    edited.target.host = host.into();
    edited
}

fn profiles_of(items: &[Profile]) -> Profiles {
    let mut profiles = Profiles::default();
    for item in items {
        profiles.upsert(item.clone());
    }
    profiles
}

/// A store and a replication over it, wired to one in-memory replica.
struct Fixture {
    store: Entity<ConnectionStore>,
    replication: Entity<ProfileReplication>,
    replica: Arc<MemoryReplica>,
    events: Arc<Mutex<Vec<ReplicationEvent>>>,
    writes: WriteLog<ReplicaState>,
}

impl Fixture {
    fn new(cx: &mut gpui::TestAppContext, local: &[Profile]) -> Self {
        Self::resumed(
            cx,
            local,
            ReplicaState::default(),
            Arc::new(MemoryReplica::new()),
        )
    }

    /// A start with `state` already agreed, as after a restart.
    fn resumed(
        cx: &mut gpui::TestAppContext,
        local: &[Profile],
        state: ReplicaState,
        replica: Arc<MemoryReplica>,
    ) -> Self {
        let local = profiles_of(local);
        let store = ConnectionStore::with_documents(local, Recents::default());
        Self::over(cx, store, state, replica)
    }

    fn over(
        cx: &mut gpui::TestAppContext,
        store: ConnectionStore,
        state: ReplicaState,
        replica: Arc<MemoryReplica>,
    ) -> Self {
        let store = cx.new(|_| store);
        let (writer, release, writes) = gated_writer().into_parts();
        release.send(Ok(())).ok();
        let document = cx.new(|_| ProfileReplication::state_document(state, writer));
        let replication = {
            let store = store.clone();
            cx.new(|cx| ProfileReplication::new(store, document, cx))
        };
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = events.clone();
        cx.update(|cx| {
            cx.subscribe(&replication, move |_, event: &ReplicationEvent, _| {
                seen.lock().unwrap().push(event.clone());
            })
            .detach();
        });
        Self {
            store,
            replication,
            replica,
            events,
            writes,
        }
    }

    fn attach(&self, cx: &mut gpui::TestAppContext) {
        let replica = self.replica.clone();
        self.replication
            .update(cx, |replication, cx| replication.attach(replica, cx));
        cx.run_until_parked();
    }

    async fn save(&self, cx: &mut gpui::TestAppContext, profile: Profile) {
        self.store
            .update(cx, |store, cx| store.save_profile(profile, cx))
            .await
            .unwrap();
        cx.run_until_parked();
    }

    async fn delete(&self, cx: &mut gpui::TestAppContext, profile: &Profile) {
        let id = profile.id;
        self.store
            .update(cx, |store, cx| store.delete_profile(id, cx))
            .await
            .unwrap();
        cx.run_until_parked();
    }

    fn local(&self, cx: &gpui::TestAppContext, profile: &Profile) -> Option<Profile> {
        let id = profile.id;
        self.store
            .read_with(cx, |store, cx| store.profiles(cx).get(id).cloned())
    }

    fn status(&self, cx: &gpui::TestAppContext) -> ReplicationStatus {
        self.replication
            .read_with(cx, |replication, _| replication.status().clone())
    }

    fn events(&self) -> Vec<ReplicationEvent> {
        self.events.lock().unwrap().clone()
    }

    /// The last agreed state handed to the writer, as a restart would load it.
    fn persisted_state(&self) -> ReplicaState {
        self.writes
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
    }

    fn place(&self, cx: &mut gpui::TestAppContext, profile: &Profile, scope: ReplicaScope) {
        let id = profile.id;
        self.replication
            .update(cx, |replication, cx| replication.place(id, scope, cx));
        cx.run_until_parked();
    }

    /// Requests a scope for a profile about to be saved, without synchronizing.
    fn stage(&self, cx: &mut gpui::TestAppContext, profile: &Profile, scope: ReplicaScope) {
        let id = profile.id;
        self.replication.update(cx, |replication, cx| {
            replication.stage(id, scope, cx);
        });
    }

    fn space_of(&self, cx: &gpui::TestAppContext, profile: &Profile) -> ReplicaScope {
        let id = profile.id;
        self.replication
            .read_with(cx, |replication, cx| replication.space_of(id, cx))
    }

    fn put_count(&self) -> usize {
        self.replica
            .writes()
            .iter()
            .filter(|call| matches!(call, ReplicaCall::Put { .. }))
            .count()
    }
}

#[gpui::test]
async fn a_new_remote_profile_is_applied_locally_without_echoing_it_back(
    cx: &mut gpui::TestAppContext,
) {
    let fixture = Fixture::new(cx, &[]);
    let remote = profile("remote");
    fixture
        .replica
        .write_remote(ReplicaScope::Personal, remote.clone());
    fixture.attach(cx);
    assert_eq!(fixture.local(cx, &remote), Some(remote.clone()));
    assert_eq!(fixture.replica.writes(), []);
    assert!(matches!(fixture.status(cx), ReplicationStatus::Synced));
}

#[gpui::test]
async fn a_new_local_profile_is_sent_once_and_its_echo_is_ignored(cx: &mut gpui::TestAppContext) {
    let local = profile("local");
    let fixture = Fixture::new(cx, std::slice::from_ref(&local));
    fixture.attach(cx);
    assert_eq!(
        fixture.replica.writes(),
        [ReplicaCall::Put {
            id: local.id,
            expected: None
        }]
    );
    assert_eq!(fixture.replica.profile(local.id), Some(local.clone()));
    let created = fixture.replica.stored(local.id).unwrap().1;
    fixture
        .replica
        .write_remote(ReplicaScope::Personal, profile("other"));
    cx.run_until_parked();
    assert_eq!(fixture.put_count(), 1);
    assert_eq!(
        fixture.persisted_state().entry(local.id).map(|e| e.version),
        Some(created)
    );
}

#[gpui::test]
async fn a_remote_edit_replaces_an_unchanged_local_profile(cx: &mut gpui::TestAppContext) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    let edited = at_host(&original, "moved.example");
    fixture
        .replica
        .write_remote(ReplicaScope::Personal, edited.clone());
    cx.run_until_parked();
    assert_eq!(fixture.local(cx, &original), Some(edited));
    assert_eq!(fixture.put_count(), 1);
    assert_eq!(fixture.events().len(), 0);
}

#[gpui::test]
async fn a_local_edit_is_sent_against_the_agreed_version(cx: &mut gpui::TestAppContext) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    let agreed = fixture.replica.stored(original.id).unwrap().1;
    let edited = at_host(&original, "moved.example");
    fixture.save(cx, edited.clone()).await;
    assert_eq!(
        fixture.replica.writes().last(),
        Some(&ReplicaCall::Put {
            id: original.id,
            expected: Some(agreed)
        })
    );
    assert_eq!(fixture.replica.profile(original.id), Some(edited));
}

#[gpui::test]
async fn the_remote_wins_when_both_sides_changed_and_the_loss_is_reported(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    fixture.replica.set_offline(true);
    cx.run_until_parked();
    let local_edit = at_host(&original, "local.example");
    fixture.save(cx, local_edit.clone()).await;
    let remote_edit = at_host(&original, "remote.example");
    fixture
        .replica
        .write_remote(ReplicaScope::Personal, remote_edit.clone());
    fixture.replica.set_offline(false);
    cx.run_until_parked();
    assert_eq!(fixture.local(cx, &original), Some(remote_edit));
    let events = fixture.events();
    assert!(matches!(
        events.as_slice(),
        [ReplicationEvent::Overwritten { id, discarded }]
            if *id == original.id && **discarded == local_edit
    ));
}

#[gpui::test]
async fn equal_edits_on_both_sides_are_agreed_without_a_write_or_an_event(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    fixture.replica.set_offline(true);
    let edited = at_host(&original, "same.example");
    fixture.save(cx, edited.clone()).await;
    let version = fixture.replica.write_remote(ReplicaScope::Personal, edited);
    fixture.replica.set_offline(false);
    cx.run_until_parked();
    assert_eq!(fixture.put_count(), 1);
    assert_eq!(fixture.events().len(), 0);
    assert_eq!(
        fixture
            .persisted_state()
            .entry(original.id)
            .map(|e| e.version),
        Some(version)
    );
}

#[gpui::test]
async fn a_local_deletion_leaves_a_tombstone_on_the_replica(cx: &mut gpui::TestAppContext) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    let agreed = fixture.replica.stored(original.id).unwrap().1;
    fixture.delete(cx, &original).await;
    assert_eq!(
        fixture.replica.writes().last(),
        Some(&ReplicaCall::Delete {
            id: original.id,
            expected: agreed
        })
    );
    assert!(matches!(
        fixture.replica.stored(original.id).map(|s| s.2),
        Some(RemoteBody::Deleted)
    ));
    assert!(fixture.persisted_state().entry(original.id).is_none());
}

#[gpui::test]
async fn a_remote_tombstone_deletes_the_unchanged_local_profile(cx: &mut gpui::TestAppContext) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    fixture
        .replica
        .delete_remote(ReplicaScope::Personal, original.id);
    cx.run_until_parked();
    assert_eq!(fixture.local(cx, &original), None);
    assert_eq!(fixture.put_count(), 1);
    assert!(fixture.persisted_state().entry(original.id).is_none());
}

#[gpui::test]
async fn a_remote_edit_restores_a_profile_deleted_locally_while_offline(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    fixture.replica.set_offline(true);
    fixture.delete(cx, &original).await;
    let edited = at_host(&original, "remote.example");
    fixture
        .replica
        .write_remote(ReplicaScope::Personal, edited.clone());
    fixture.replica.set_offline(false);
    cx.run_until_parked();
    assert_eq!(fixture.local(cx, &original), Some(edited));
    assert!(
        fixture
            .replica
            .writes()
            .iter()
            .all(|call| !matches!(call, ReplicaCall::Delete { .. }))
    );
}

#[gpui::test]
async fn a_local_edit_is_sent_over_a_remote_tombstone(cx: &mut gpui::TestAppContext) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    fixture.replica.set_offline(true);
    let edited = at_host(&original, "local.example");
    fixture.save(cx, edited.clone()).await;
    let tombstone = fixture
        .replica
        .delete_remote(ReplicaScope::Personal, original.id);
    fixture.replica.set_offline(false);
    cx.run_until_parked();
    assert_eq!(
        fixture.replica.writes().last(),
        Some(&ReplicaCall::Put {
            id: original.id,
            expected: Some(tombstone)
        })
    );
    assert_eq!(fixture.replica.profile(original.id), Some(edited.clone()));
    assert_eq!(fixture.local(cx, &original), Some(edited));
}

//! Native repository/publication integration; no adapter dependency enters the feature.
use super::*;
use gpui_kit::AppContext as _;
use nocterm_connections::{Connections, ConnectionsError, EditorInputs};
use nocterm_connections_model::{
    draft::EditorDefaults,
    profiles::{Profile, Profiles},
    recents::{Recent, Recents},
    value_ports::SuppliedValues,
};
use nocterm_core::persist;
use nocterm_session::{Auth, CredentialId, Target};
use nocterm_store::{ShutdownCoordinator, StoreError};
use std::fs;
fn owner(paths: &Paths) -> Connections {
    let repository = Arc::new(FileConnectionRepository::new(
        paths.connections_file(),
        paths.recents_file(),
        paths.state_dir().join("servers.toml"),
    ));
    let documents = repository.load();
    let profiles = repository.clone();
    let recents = repository;
    let values = Arc::new(SuppliedValues::new(1, 0));
    Connections::prepared(
        documents.profiles,
        documents.recents,
        Arc::new(move |snapshot| {
            let repository = profiles.clone();
            Box::pin(async move {
                repository
                    .save_profiles(&snapshot)
                    .map_err(StoreError::backend)
            })
        }),
        Arc::new(move |snapshot| {
            let repository = recents.clone();
            Box::pin(async move {
                repository
                    .save_recents(&snapshot)
                    .map_err(StoreError::backend)
            })
        }),
        documents.read_only,
        EditorInputs::new(EditorDefaults::default(), values.clone()),
        values,
    )
}
fn profile(name: &str) -> Profile {
    Profile {
        id: nocterm_connections_model::test_support::profile_id_for(name),
        name: name.into(),
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
fn documents(items: &[Profile]) -> Profiles {
    let mut profiles = Profiles::default();
    for item in items {
        profiles.upsert(item.clone());
    }
    profiles
}
fn recent_profiles(items: &[Profile]) -> Recents {
    let mut recents = Recents::default();
    for profile in items {
        recents.record(Recent {
            options: profile.options.clone(),
            target: profile.target.clone(),
            auth: profile.auth.clone(),
            profile: Some(profile.id),
            last_used: 0,
            credential: profile.credential,
            launch: profile.launch.clone(),
        });
    }
    recents
}

#[gpui_kit::test]
async fn associating_unmatched_credentials_preserves_malformed_profile_bytes(
    cx: &mut gpui_kit::TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::rooted_at(directory.path());
    fs::create_dir_all(paths.config_dir()).unwrap();
    let damaged = b"# preserve this user document\ninvalid [[";
    fs::write(paths.connections_file(), damaged).unwrap();
    let connections = cx.new(|_| owner(&paths));
    assert!(connections.read_with(cx, |c, _| c.load_error().is_some()));
    assert!(connections.read_with(cx, |c, cx| c.profiles(cx).is_empty()));
    let result = connections
        .update(cx, |c, cx| {
            c.associate_credential(
                &Target::new("unmatched", "host.test", 22),
                &Auth::Password,
                CredentialId::from_bytes([8; 16]),
                cx,
            )
        })
        .await;
    assert_eq!(fs::read(paths.connections_file()).unwrap(), damaged);
    assert!(matches!(result, Err(ConnectionsError::ReadOnly(_))));
}

#[gpui_kit::test]
async fn stale_readonly_load_cannot_overwrite_profiles_repaired_by_another_writer(
    cx: &mut gpui_kit::TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::rooted_at(directory.path());
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(paths.connections_file(), "invalid [[").unwrap();
    let connections = cx.new(|_| owner(&paths));
    assert!(connections.read_with(cx, |c, _| c.load_error().is_some()));
    let repaired = profile("Repaired externally");
    persist::save(
        &paths.connections_file(),
        &documents(std::slice::from_ref(&repaired)),
    )
    .unwrap();
    let preserved = fs::read(paths.connections_file()).unwrap();
    let result = connections
        .update(cx, |c, cx| {
            c.associate_credential(
                &repaired.target,
                &repaired.auth,
                CredentialId::from_bytes([9; 16]),
                cx,
            )
        })
        .await;
    assert_eq!(fs::read(paths.connections_file()).unwrap(), preserved);
    assert!(matches!(result, Err(ConnectionsError::ReadOnly(_))));
    let reloaded = cx.new(|_| owner(&paths));
    assert_eq!(
        reloaded.read_with(cx, |c, cx| c.profiles(cx).get(repaired.id).cloned()),
        Some(repaired)
    );
}

#[gpui_kit::test]
async fn capturing_readonly_credential_association_preserves_profiles_and_recent_credentials(
    cx: &mut gpui_kit::TestAppContext,
) {
    for externally_repaired in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(directory.path());
        fs::create_dir_all(paths.config_dir()).unwrap();
        fs::write(paths.connections_file(), "# readonly user data\ninvalid [[").unwrap();
        let mut original = profile("Externally repaired");
        original.credential = Some(CredentialId::from_bytes([4; 16]));
        let recents = recent_profiles(std::slice::from_ref(&original));
        persist::save(&paths.recents_file(), &recents).unwrap();
        let coordinator = ShutdownCoordinator::default();
        let connections = cx.update(|cx| owner(&paths).into_entity(&coordinator, cx).unwrap());
        assert!(connections.read_with(cx, |c, _| c.load_error().is_some()));
        if externally_repaired {
            persist::save(
                &paths.connections_file(),
                &documents(std::slice::from_ref(&original)),
            )
            .unwrap();
        }
        let profiles_before = fs::read(paths.connections_file()).unwrap();
        let recents_before = fs::read(paths.recents_file()).unwrap();
        let receipt = connections.update(cx, |c, cx| {
            c.associate_credential(
                &original.target,
                &original.auth,
                CredentialId::from_bytes([5; 16]),
                cx,
            )
        });
        let captured = cx.update(|cx| coordinator.capture_all(cx));
        drop(connections);
        let report = cx.background_executor.spawn(captured).await;
        assert!(report.failures().is_empty());
        assert_eq!(fs::read(paths.recents_file()).unwrap(), recents_before);
        assert_eq!(fs::read(paths.connections_file()).unwrap(), profiles_before);
        assert!(matches!(receipt.await, Err(ConnectionsError::ReadOnly(_))));
        let saved: Recents = persist::load(&paths.recents_file()).unwrap().unwrap();
        assert_eq!(saved.iter().next().unwrap().credential, original.credential);
    }
}
#[gpui_kit::test]
async fn readonly_broken_profiles_and_unknown_toml_are_preserved(
    cx: &mut gpui_kit::TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::rooted_at(dir.path());
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(paths.connections_file(), "invalid [[").unwrap();
    let entity = cx.new(|_| owner(&paths));
    assert!(matches!(
        entity
            .update(cx, |c, cx| c.save_profile(profile("New"), cx))
            .await,
        Err(ConnectionsError::ReadOnly(_))
    ));
    assert_eq!(
        fs::read_to_string(paths.connections_file()).unwrap(),
        "invalid [["
    );
    fs::write(
        paths.connections_file(),
        "# user comment\nfolders = [\"Work\"] # folder comment\n[plugin]\nunknown = 7\n",
    )
    .unwrap();
    let entity = cx.new(|_| owner(&paths));
    entity
        .update(cx, |c, cx| c.save_profile(profile("New"), cx))
        .await
        .unwrap();
    let text = fs::read_to_string(paths.connections_file()).unwrap();
    assert!(
        text.contains("# user comment")
            && text.contains("unknown = 7")
            && text.contains("# folder comment")
    );
}

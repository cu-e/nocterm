use super::*;
use nocterm_connections_model::{
    profiles::{Profile, ProfileId},
    recents::Recent,
};
use nocterm_session::{Auth, Target};
use std::error::Error as _;
fn repository(directory: &std::path::Path) -> FileConnectionRepository {
    FileConnectionRepository::new(
        directory.join("profiles.toml"),
        directory.join("recents.toml"),
        directory.join("servers.toml"),
    )
}
fn profile() -> Profile {
    Profile {
        id: ProfileId::from_bytes([7; 16]),
        name: "Build".into(),
        description: "notes".into(),
        target: Target::new("tester", "build.test", 22),
        options: Default::default(),
        auth: Auth::Auto,
        credential: None,
        group: Some("Work".into()),
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        jump_hosts: Vec::new(),
    }
}
#[test]
fn three_documents_round_trip_without_format_migration() {
    let directory = tempfile::tempdir().unwrap();
    let repository = repository(directory.path());
    let profile = profile();
    let mut profiles = Profiles::default();
    profiles.upsert(profile.clone());
    let mut recents = Recents::default();
    recents.record(Recent {
        target: profile.target.clone(),
        auth: profile.auth.clone(),
        options: profile.options.clone(),
        profile: Some(profile.id),
        last_used: 42,
        credential: None,
        launch: None,
    });
    let mut known = Known::default();
    known.set_os(
        profile.id,
        nocterm_connections_model::os::OsId::new("debian").unwrap(),
    );
    repository.save_profiles(&profiles).unwrap();
    repository.save_recents(&recents).unwrap();
    repository.save_facts(&known).unwrap();
    let loaded = repository.load();
    assert_eq!(loaded.profiles, profiles);
    assert_eq!(loaded.recents, recents);
    assert_eq!(loaded.known, known);
    assert!(loaded.read_only.is_none());
}
#[test]
fn malformed_profiles_stay_readonly_while_bookkeeping_rebuilds() {
    let directory = tempfile::tempdir().unwrap();
    let repository = repository(directory.path());
    std::fs::write(directory.path().join("profiles.toml"), "invalid [[").unwrap();
    std::fs::write(directory.path().join("recents.toml"), "invalid [[").unwrap();
    std::fs::write(directory.path().join("servers.toml"), "invalid [[").unwrap();
    let loaded = repository.load();
    let error = loaded.read_only.unwrap();
    assert_eq!(error.document(), ConnectionDocument::Profiles);
    assert_eq!(error.operation(), StorageOperation::Read);
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<persist::PersistError>()
            .is_some()
    );
    assert!(loaded.profiles.is_empty());
    assert!(loaded.recents.iter().next().is_none());
    assert_eq!(loaded.known, Known::default());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("profiles.toml")).unwrap(),
        "invalid [["
    );
}
#[test]
fn retained_top_level_tables_and_comments_survive_profile_publication() {
    let directory = tempfile::tempdir().unwrap();
    let repository = repository(directory.path());
    std::fs::write(
        directory.path().join("profiles.toml"),
        "# user comment\nfolders = [\"Work\"] # folder comment\n[plugin]\nunknown = 7\n",
    )
    .unwrap();
    let mut loaded = repository.load();
    loaded.profiles.upsert(profile());
    repository.save_profiles(&loaded.profiles).unwrap();
    let text = std::fs::read_to_string(directory.path().join("profiles.toml")).unwrap();
    assert!(
        text.contains("# user comment")
            && text.contains("# folder comment")
            && text.contains("unknown = 7")
    );
}
#[cfg(unix)]
#[test]
fn existing_configuration_symlink_keeps_its_target_publication_policy() {
    let directory = tempfile::tempdir().unwrap();
    let repository = repository(directory.path());
    let target = directory.path().join("dotfiles.toml");
    std::fs::write(&target, "# linked config\n").unwrap();
    std::os::unix::fs::symlink(&target, directory.path().join("profiles.toml")).unwrap();
    let mut profiles = Profiles::default();
    profiles.upsert(profile());
    repository.save_profiles(&profiles).unwrap();
    assert!(
        std::fs::symlink_metadata(directory.path().join("profiles.toml"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        persist::load::<Profiles>(&target).unwrap().unwrap(),
        profiles
    );
    assert!(
        std::fs::read_to_string(target)
            .unwrap()
            .contains("# linked config")
    );
}

#[test]
fn missing_documents_load_as_empty_without_readonly_diagnostic() {
    let directory = tempfile::tempdir().unwrap();
    let loaded = repository(directory.path()).load();
    assert!(loaded.profiles.is_empty());
    assert!(loaded.read_only.is_none());
    assert_eq!(loaded.recents, Recents::default());
    assert_eq!(loaded.known, Known::default());
}
#[test]
fn malformed_profile_shape_keeps_native_path_diagnostic() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("profiles.toml"),
        "[[connection]]\nname = 3\n",
    )
    .unwrap();
    let loaded = repository(directory.path()).load();
    assert!(loaded.profiles.is_empty());
    assert!(
        loaded
            .read_only
            .unwrap()
            .to_string()
            .contains("profiles.toml")
    );
}
#[test]
fn malformed_recents_rebuild_without_disabling_profile_writes() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("recents.toml"), "not toml at all [").unwrap();
    let loaded = repository(directory.path()).load();
    assert_eq!(loaded.recents, Recents::default());
    assert!(loaded.read_only.is_none());
}

#[test]
fn jump_hosts_of_both_forms_survive_a_save_and_load() {
    use nocterm_connections_model::jump_route::JumpHostEntry;
    let directory = tempfile::tempdir().unwrap();
    let repository = repository(directory.path());
    let mut profile = profile();
    profile.jump_hosts = vec![
        JumpHostEntry::Profile(ProfileId::from_bytes([9; 16])),
        JumpHostEntry::Address(Target::new("ops", "gw.test", 2222)),
    ];
    let mut profiles = Profiles::default();
    profiles.upsert(profile);
    repository.save_profiles(&profiles).unwrap();
    assert_eq!(repository.load().profiles, profiles);
}

#[test]
fn a_document_without_jump_hosts_keeps_its_bytes_after_a_save() {
    let directory = tempfile::tempdir().unwrap();
    let repository = repository(directory.path());
    let mut profiles = Profiles::default();
    profiles.upsert(profile());
    repository.save_profiles(&profiles).unwrap();
    let first = std::fs::read(directory.path().join("profiles.toml")).unwrap();
    assert!(!String::from_utf8_lossy(&first).contains("jump_hosts"));
    repository
        .save_profiles(&repository.load().profiles)
        .unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("profiles.toml")).unwrap(),
        first
    );
}

use std::{error::Error as _, fs};

use nocterm_project_model::{
    Catalog, CatalogError, HostLocator, MAX_CATALOG_BYTES, MAX_PROJECTS, ProjectDefinition,
    ProjectDraft, ProjectId, ProjectMetadataSource, ProjectRepository, ProjectSource,
    ProjectStorageError, RootPath,
};
use nocterm_project_storage::{FileProjectRepository, NativeProjectMetadata};

fn catalog(value: u8) -> Catalog {
    Catalog::new(vec![
        ProjectDefinition::try_from(ProjectDraft {
            id: ProjectId::from_bytes([value; 16]),
            name: "Project".into(),
            host: HostLocator::Local,
            root: RootPath::parse("/srv/project").unwrap(),
            source: ProjectSource::Manual,
            updated: 123,
        })
        .unwrap(),
    ])
    .unwrap()
}

#[test]
fn missing_catalog_is_absent_without_creating_files_or_parents() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing/projects.json");
    let repository = FileProjectRepository::new(path.clone());
    assert_eq!(repository.load().unwrap(), None);
    assert!(!path.parent().unwrap().exists());
}

#[test]
fn saved_catalog_round_trips_checked_identity_and_creates_private_parent_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("nested/projects.json");
    let repository = FileProjectRepository::new(path.clone());
    repository.save(&catalog(1)).unwrap();
    assert_eq!(repository.load().unwrap(), Some(catalog(1)));
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn loading_a_legacy_catalog_preserves_identity_and_republishes_only_folder_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projects.json");
    let expected = catalog(1);
    let mut old: serde_json::Value = serde_json::from_slice(&expected.encode().unwrap()).unwrap();
    old["projects"][0]["policy"] = serde_json::json!({"version":1,"execution":false});
    fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    let repository = FileProjectRepository::new(path.clone());
    let loaded = repository.load().unwrap().unwrap();
    assert_eq!(loaded, expected);
    repository.save(&loaded).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert!(saved["projects"][0].get("policy").is_none());
    assert_eq!(saved["projects"][0]["id"], old["projects"][0]["id"]);
}

#[test]
fn malformed_unsupported_and_oversized_files_are_typed_refusals_without_rewrite() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projects.json");
    let repository = FileProjectRepository::new(path.clone());
    for (bytes, version) in [
        (b"{".as_slice(), false),
        (br#"{"version":99,"projects":[]}"#, true),
    ] {
        fs::write(&path, bytes).unwrap();
        let error = repository.load().unwrap_err();
        if version {
            assert!(matches!(
                error,
                ProjectStorageError::InvalidDocument(CatalogError::UnsupportedVersion(99))
            ));
        } else {
            assert!(matches!(
                error,
                ProjectStorageError::InvalidDocument(CatalogError::Codec(_))
            ));
        }
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::write(&path, vec![b' '; MAX_CATALOG_BYTES + 1]).unwrap();
    assert!(matches!(
        repository.load(),
        Err(ProjectStorageError::InvalidDocument(CatalogError::TooLarge))
    ));
}

#[test]
fn catalog_capacity_and_invalid_names_retain_typed_validation_causes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projects.json");
    let repository = FileProjectRepository::new(path.clone());
    let project = catalog(1).iter().next().unwrap().to_draft();
    let oversized =
        serde_json::json!({"version":1, "projects": vec![project.clone(); MAX_PROJECTS + 1]});
    fs::write(&path, serde_json::to_vec(&oversized).unwrap()).unwrap();
    assert!(matches!(
        repository.load(),
        Err(ProjectStorageError::InvalidDocument(CatalogError::Capacity))
    ));
    let mut invalid = project;
    invalid.name.clear();
    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"version":1,"projects":[invalid]})).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        repository.load(),
        Err(ProjectStorageError::InvalidDocument(CatalogError::Invalid(
            nocterm_project_model::ProjectValidationError::InvalidName
        )))
    ));
}

#[test]
fn empty_files_and_directories_are_errors_rather_than_missing_catalogs() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projects.json");
    fs::write(&path, b"").unwrap();
    let repository = FileProjectRepository::new(path.clone());
    assert!(matches!(
        repository.load(),
        Err(ProjectStorageError::InvalidDocument(CatalogError::Codec(_)))
    ));
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(matches!(
        repository.load(),
        Err(ProjectStorageError::Read { .. })
    ));
}

#[test]
fn failed_reads_retain_the_original_io_classification() {
    let directory = tempfile::tempdir().unwrap();
    let parent_file = directory.path().join("file");
    fs::write(&parent_file, b"preserve").unwrap();
    let error = FileProjectRepository::new(parent_file.join("projects.json"))
        .load()
        .unwrap_err();
    assert!(matches!(error, ProjectStorageError::Read { .. }));
    let original = error
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_ne!(original.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(fs::read(parent_file).unwrap(), b"preserve");
}

#[test]
fn concurrent_readers_observe_complete_old_or_new_catalogs_during_publication() {
    use std::sync::{Arc, Barrier};
    let directory = tempfile::tempdir().unwrap();
    let repository = Arc::new(FileProjectRepository::new(
        directory.path().join("projects.json"),
    ));
    let first = catalog(1);
    let second = catalog(2);
    repository.save(&first).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let writer_repository = repository.clone();
    let writer_barrier = barrier.clone();
    let writer = std::thread::spawn(move || {
        writer_barrier.wait();
        for n in 0..20 {
            writer_repository.save(&catalog(n % 2 + 1)).unwrap();
        }
    });
    barrier.wait();
    for _ in 0..100 {
        let loaded = repository.load().unwrap().unwrap();
        assert!(loaded == first || loaded == second);
    }
    writer.join().unwrap();
}

#[test]
fn malformed_catalog_keeps_the_original_json_error_in_its_source_chain() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projects.json");
    fs::write(&path, b"{").unwrap();
    let error = FileProjectRepository::new(path).load().unwrap_err();
    let mut source: &(dyn std::error::Error + 'static) = &error;
    while let Some(next) = source.source() {
        source = next;
        if source.downcast_ref::<serde_json::Error>().is_some() {
            return;
        }
    }
    panic!("original serde_json error missing");
}

#[test]
fn failed_publication_preserves_existing_destination_and_cleans_staging() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projects.json");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("existing"), b"preserve").unwrap();
    let error = FileProjectRepository::new(path.clone())
        .save(&catalog(1))
        .unwrap_err();
    assert!(matches!(error, ProjectStorageError::Publish { .. }));
    assert_eq!(fs::read(path.join("existing")).unwrap(), b"preserve");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    let core = error.source().unwrap().source().unwrap();
    assert!(core.downcast_ref::<nocterm_core::fs::FileError>().is_some());
    assert!(
        core.source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .is_some()
    );
}

#[test]
fn parent_creation_failure_retains_original_file_and_native_error() {
    let directory = tempfile::tempdir().unwrap();
    let existing = directory.path().join("existing");
    fs::write(&existing, b"preserve").unwrap();
    let repository = FileProjectRepository::new(existing.join("projects.json"));
    assert!(matches!(
        repository.save(&catalog(1)),
        Err(ProjectStorageError::Publish { .. })
    ));
    assert_eq!(fs::read(existing).unwrap(), b"preserve");
}

#[cfg(unix)]
#[test]
fn publication_replaces_a_final_symlink_and_preserves_the_target() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("other.json");
    let path = directory.path().join("projects.json");
    fs::write(&target, b"private unrelated file").unwrap();
    symlink(&target, &path).unwrap();
    let repository = FileProjectRepository::new(path.clone());
    repository.save(&catalog(1)).unwrap();
    assert_eq!(fs::read(target).unwrap(), b"private unrelated file");
    assert_eq!(repository.load().unwrap(), Some(catalog(1)));
    assert!(!fs::symlink_metadata(path).unwrap().file_type().is_symlink());
}

#[cfg(unix)]
#[test]
fn catalog_reads_refuse_regular_and_dangling_final_symlinks() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("other.json");
    let path = directory.path().join("projects.json");
    fs::write(&target, catalog(1).encode().unwrap()).unwrap();
    symlink(&target, &path).unwrap();
    let repository = FileProjectRepository::new(path);
    assert!(matches!(
        repository.load(),
        Err(ProjectStorageError::Read { .. })
    ));
    fs::remove_file(&target).unwrap();
    assert!(matches!(
        repository.load(),
        Err(ProjectStorageError::Read { .. })
    ));
    repository.save(&catalog(2)).unwrap();
    assert_eq!(repository.load().unwrap(), Some(catalog(2)));
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn named_pipes_are_refused_without_waiting_for_a_writer() {
    use std::{sync::mpsc, time::Duration};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projects.json");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &path,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    let (send, receive) = mpsc::sync_channel(1);
    let worker =
        std::thread::spawn(move || send.send(FileProjectRepository::new(path).load()).unwrap());
    let result = receive
        .recv_timeout(Duration::from_secs(5))
        .expect("catalog FIFO read blocked");
    worker.join().unwrap();
    assert!(matches!(result, Err(ProjectStorageError::Read { .. })));
}

#[cfg(unix)]
#[test]
fn every_publication_resets_world_readable_file_permissions_to_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projects.json");
    let repository = FileProjectRepository::new(path.clone());
    fs::write(&path, b"old").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    repository.save(&catalog(1)).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    repository.save(&catalog(2)).unwrap();
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn native_metadata_allocates_fresh_checked_project_ids() {
    let source = NativeProjectMetadata;
    let first = source.new_identity().unwrap();
    let second = source.new_identity().unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(first.id.to_string().parse::<ProjectId>().unwrap(), first.id);
}

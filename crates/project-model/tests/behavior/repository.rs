use std::{path::PathBuf, sync::Arc};

use nocterm_project_model::{
    Catalog, ProjectId, ProjectMetadataSource, ProjectRepository, ProjectRepositoryOperation,
    ProjectStorageError,
    testing::{InMemoryProjectRepository, SequenceProjectMetadata},
};

use crate::support::definition;

#[test]
fn memory_repository_distinguishes_absence_from_a_published_empty_catalog() {
    let repository = InMemoryProjectRepository::new(None);
    assert_eq!(repository.load().unwrap(), None);
    repository.save(&Catalog::default()).unwrap();
    assert_eq!(repository.load().unwrap(), Some(Catalog::default()));
}

#[test]
fn a_failed_publication_preserves_the_previous_memory_document_and_original_cause() {
    use std::error::Error as _;
    let old = Catalog::new(vec![definition(1)]).unwrap();
    let next = Catalog::new(vec![definition(2)]).unwrap();
    let repository = InMemoryProjectRepository::new(Some(old.clone()));
    repository.fail_next(
        ProjectRepositoryOperation::Save,
        ProjectStorageError::publish(PathBuf::from("projects.json"), std::fmt::Error),
    );
    let error = repository.save(&next).unwrap_err();
    assert!(matches!(error, ProjectStorageError::Publish { .. }));
    assert!(
        error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::fmt::Error>()
            .is_some()
    );
    assert_eq!(repository.load().unwrap(), Some(old));
    repository.save(&next).unwrap();
    assert_eq!(repository.load().unwrap(), Some(next));
}

#[test]
fn paused_publication_changes_the_document_only_after_the_explicit_gate_release() {
    let old = Catalog::new(vec![definition(1)]).unwrap();
    let next = Catalog::new(vec![definition(2)]).unwrap();
    let repository = Arc::new(InMemoryProjectRepository::new(Some(old.clone())));
    let mut pause = repository.pause_next(ProjectRepositoryOperation::Save);
    let worker_repository = repository.clone();
    let candidate = next.clone();
    let worker = std::thread::spawn(move || worker_repository.save(&candidate));
    assert!(futures::executor::block_on(pause.entered()));
    assert_eq!(repository.snapshot(), Some(old));
    pause.release();
    worker.join().unwrap().unwrap();
    assert_eq!(repository.load().unwrap(), Some(next));
}

#[test]
fn scripted_read_failure_is_distinct_from_an_absent_document() {
    let repository = InMemoryProjectRepository::new(None);
    repository.fail_next(
        ProjectRepositoryOperation::Load,
        ProjectStorageError::read(PathBuf::from("projects.json"), std::fmt::Error),
    );
    assert!(matches!(
        repository.load(),
        Err(ProjectStorageError::Read { .. })
    ));
    assert_eq!(repository.load().unwrap(), None);
}

#[test]
fn supplied_metadata_advances_identity_only_on_allocation_and_uses_supplied_time() {
    let metadata = SequenceProjectMetadata::new(17, 123);
    assert_eq!(metadata.timestamp(), 123);
    let first = metadata.new_identity().unwrap();
    metadata.set_timestamp(456);
    let second = metadata.new_identity().unwrap();
    assert_eq!(first.id, ProjectId::from_bytes(17u128.to_be_bytes()));
    assert_eq!(first.created, 123);
    assert_eq!(second.id, ProjectId::from_bytes(18u128.to_be_bytes()));
    assert_eq!(second.created, 456);
}

#[test]
fn a_failed_metadata_allocation_keeps_its_original_cause_and_issues_no_identity() {
    use nocterm_project_model::ProjectMetadataError;
    use std::error::Error as _;
    let metadata = SequenceProjectMetadata::new(17, 123);
    metadata.fail_next(ProjectMetadataError::entropy(std::fmt::Error));
    let error = metadata.new_identity().unwrap_err();
    assert!(matches!(error, ProjectMetadataError::Entropy(_)));
    assert!(
        error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::fmt::Error>()
            .is_some()
    );
    assert_eq!(
        metadata.new_identity().unwrap().id,
        ProjectId::from_bytes(17u128.to_be_bytes())
    );
}

#[test]
fn an_exhausted_supplied_sequence_refuses_instead_of_repeating_identity() {
    let metadata = SequenceProjectMetadata::new(u128::MAX, 123);
    assert_eq!(
        metadata.new_identity().unwrap().id,
        ProjectId::from_bytes([u8::MAX; 16])
    );
    assert!(matches!(
        metadata.new_identity(),
        Err(nocterm_project_model::ProjectMetadataError::Exhausted)
    ));
}

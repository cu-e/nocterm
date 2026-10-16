use super::*;
use nocterm_core::persist::{self, PersistError};
use nocterm_snippets::{
    Language, Library, Snippet, SnippetIds, SnippetRepository, StorageOperation,
};
use std::error::Error;
fn library() -> Library {
    Library {
        snippets: vec![
            Snippet {
                name: "Deploy 日本語".into(),
                description: "Описание\nline two".into(),
                content: "#!/bin/bash\nprintf '%s\\n' 'привет'\n\n".into(),
                language: Language::Bash,
                profiles: vec!["00000000-0000-0000-0000-000000000123".into()],
                groups: vec!["prod 日本語".into()],
                ..Snippet::new(uuid::Uuid::from_u128(1))
            },
            Snippet {
                name: "Default bindings".into(),
                content: "echo simple\n".into(),
                ..Snippet::new(uuid::Uuid::from_u128(2))
            },
        ],
    }
}
/// Original production snippets writer operation pinned at reviewed80929 model.rs.
/// This oracle deliberately uses save, which rewrites existing TOML comments.
fn original_writer(path: &std::path::Path, library: &Library) -> Result<(), PersistError> {
    persist::save(path, library)
}
#[test]
fn native_publication_matches_the_original_writer_bytes_and_rewrites_comments() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old/snippets.toml");
    let current = directory.path().join("new/snippets.toml");
    let value = library();
    original_writer(&old, &value).unwrap();
    let repository = FileSnippetRepository::new(current.clone());
    repository.save(&value).unwrap();
    let expected = std::fs::read(&old).unwrap();
    assert_eq!(std::fs::read(&current).unwrap(), expected);
    std::fs::write(
        &current,
        format!(
            "# user comment\n{}",
            std::str::from_utf8(&expected).unwrap()
        ),
    )
    .unwrap();
    assert_eq!(repository.load().unwrap(), Some(value.clone()));
    repository.save(&value).unwrap();
    assert_eq!(std::fs::read(&current).unwrap(), expected);
    assert!(
        !std::fs::read_to_string(&current)
            .unwrap()
            .contains("# user comment")
    );
}
#[test]
fn missing_corrupt_and_invalid_libraries_keep_distinct_acquisition_results() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snippets.toml");
    let repository = FileSnippetRepository::new(path.clone());
    assert_eq!(repository.load().unwrap(), None);
    std::fs::write(&path, "snippets = =").unwrap();
    let error = repository.load().unwrap_err();
    assert_eq!(error.operation(), StorageOperation::Read);
    let native = error
        .source()
        .unwrap()
        .downcast_ref::<PersistError>()
        .unwrap();
    let PersistError::Parse { source, .. } = native else {
        panic!("original parse error retained")
    };
    let original = native.source().unwrap().downcast_ref().unwrap();
    assert!(std::ptr::eq(original, source.as_ref()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "snippets = =");
    let mut invalid = library();
    invalid.snippets[0].content.clear();
    repository.save(&invalid).unwrap();
    let loaded = repository.load().unwrap().unwrap();
    assert_eq!(loaded, invalid, "adapter does not own validation/fallback");
    assert!(loaded.validate().is_err());
}
#[test]
fn native_write_refusals_retain_the_original_io_source() {
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("file");
    std::fs::write(&parent, "blocking parent").unwrap();
    for path in [parent.join("snippets.toml"), directory.path().to_path_buf()] {
        let error = FileSnippetRepository::new(path)
            .save(&library())
            .unwrap_err();
        assert_eq!(error.operation(), StorageOperation::Write);
        let native = error
            .source()
            .unwrap()
            .downcast_ref::<PersistError>()
            .unwrap();
        let PersistError::Io { source, .. } = native else {
            panic!("original IO error retained")
        };
        let original = native.source().unwrap().downcast_ref().unwrap();
        assert!(std::ptr::eq(original, source));
    }
    assert_eq!(std::fs::read_to_string(parent).unwrap(), "blocking parent");
}
#[test]
fn native_identity_is_generated_only_when_the_supplied_port_is_called() {
    let ids = NativeSnippetIds;
    let first = ids.next();
    let second = ids.next();
    assert_eq!(first.get_version_num(), 4);
    assert_eq!(second.get_version_num(), 4);
    assert_ne!(first, second);
}

use super::*;

fn values(text: &str) -> toml::Table {
    toml::from_str(text).unwrap()
}

#[test]
fn independent_owners_cannot_replace_a_newer_owned_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    fs::write(&path, "[terminal]\nsize = 14\n").unwrap();
    let first = RevisionedFile::new(&path);
    let second = RevisionedFile::new(&path);
    let _: toml::Table = first.load().unwrap().unwrap();
    let _: toml::Table = second.load().unwrap().unwrap();
    first
        .save_preserving(&values("[terminal]\nsize = 15\n"))
        .unwrap();
    assert!(matches!(
        second.save_preserving(&values("[terminal]\nsize = 16\n")),
        Err(PersistError::Conflict { .. })
    ));
    assert!(fs::read_to_string(&path).unwrap().contains("size = 15"));
    let _: toml::Table = second.load().unwrap().unwrap();
    second
        .save_preserving(&values("[terminal]\nsize = 16\n"))
        .unwrap();
}

#[test]
fn unowned_external_tables_survive_while_owned_edits_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    fs::write(&path, "[future]\nflag = true\n").unwrap();
    let file = RevisionedFile::new(&path);
    let _: toml::Table = file.load().unwrap().unwrap();
    fs::write(&path, "[future]\n# retain this comment\nflag = false\n").unwrap();
    file.save_preserving_owned(&values("[terminal]\nsize = 15\n"), |key| key == "terminal")
        .unwrap();
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("# retain this comment\nflag = false")
    );
    fs::write(&path, "[terminal]\nsize = 18\n").unwrap();
    assert!(matches!(
        file.save_preserving_owned(&values("[terminal]\nsize = 16\n"), |key| key == "terminal"),
        Err(PersistError::Conflict { .. })
    ));
}

#[test]
fn clones_share_a_publication_revision_and_missing_file_creation_is_observed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new/settings.toml");
    let file = RevisionedFile::new(&path);
    assert!(file.load::<toml::Table>().unwrap().is_none());
    let clone = file.clone();
    file.save_preserving(&values("a = 1")).unwrap();
    clone.save_preserving(&values("a = 2")).unwrap();
    let other = RevisionedFile::new(&path);
    other.load::<toml::Table>().unwrap();
    clone.save_preserving(&values("a = 3")).unwrap();
    assert!(matches!(
        other.save_preserving(&values("a = 4")),
        Err(PersistError::Conflict { .. })
    ));
}

#[cfg(unix)]
#[test]
fn symlink_aliases_use_the_same_lock_and_lock_symlinks_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    let alias = dir.path().join("alias.toml");
    fs::write(&path, "a = 1").unwrap();
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    let file = RevisionedFile::new(&alias);
    file.load::<toml::Table>().unwrap();
    let lock = publication_lock(&path).unwrap();
    assert!(file.save_preserving(&values("a = 2")).is_err());
    drop(lock);
    file.save_preserving(&values("a = 2")).unwrap();
    assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
    fs::remove_file(path.with_file_name("settings.toml.lock")).unwrap();
    let victim = dir.path().join("victim");
    fs::write(&victim, "untouched").unwrap();
    std::os::unix::fs::symlink(&victim, path.with_file_name("settings.toml.lock")).unwrap();
    assert!(file.save_preserving(&values("a = 3")).is_err());
    assert_eq!(fs::read_to_string(victim).unwrap(), "untouched");
}

#[test]
fn serialization_refusal_does_not_touch_an_unreadable_document_or_create_a_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    fs::write(&path, "bad = =").unwrap();
    let file = RevisionedFile::new(&path);
    assert!(matches!(
        file.save_preserving(&42u32),
        Err(PersistError::Serialize { .. })
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), "bad = =");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(matches!(
        super::super::save_preserving(&path, &42u32),
        Err(PersistError::Serialize { .. })
    ));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

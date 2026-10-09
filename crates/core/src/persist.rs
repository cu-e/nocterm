//! Reading and writing TOML files without ever leaving a half-written one.
//!
//! Writes go to a temporary file in the same directory and are renamed over
//! the destination, so a crash or a full disk leaves the previous contents
//! intact.

use std::{
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use serde::{Serialize, de::DeserializeOwned};
use toml_edit::{DocumentMut, Item, Table};

pub mod queue;

pub use queue::{InFlight, Rejected, WriteQueue, Writing};

/// A file could not be read, understood or written.
#[derive(Debug, thiserror::Error)]
pub enum PersistError {
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("{}: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    #[error("{}: {source}", path.display())]
    Serialize {
        path: PathBuf,
        source: Box<toml::ser::Error>,
    },
}

impl PersistError {
    fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// Reads `path`. A file that does not exist is `None`, not an error.
pub fn load<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, PersistError> {
    let Some(text) = read(path)? else {
        return Ok(None);
    };
    toml::from_str(&text)
        .map(Some)
        .map_err(|source| PersistError::Parse {
            path: path.to_path_buf(),
            source: Box::new(source),
        })
}

/// Replaces the contents of `path` with `value`.
///
/// Use this for files nocterm owns outright; anything a user edits by hand
/// should go through [`save_preserving`].
pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), PersistError> {
    let text = serialize(path, value)?;
    write_atomic(path, &text).map_err(|source| PersistError::io(path, source))
}

/// Replaces the contents of `path` with `contents`, atomically and readable
/// only by the user, for files nocterm owns that are not TOML.
pub fn save_text(path: &Path, contents: &str) -> Result<(), PersistError> {
    publish_atomic(path, contents).map_err(|source| PersistError::io(path, source))
}

/// Writes `value` to `path`, keeping the comments and key order already in
/// the file.
///
/// Keys `value` no longer produces are removed. A file that cannot be parsed
/// is first copied to `<name>.bak`, so a hand-edited file is never lost to a
/// syntax error.
pub fn save_preserving<T: Serialize>(path: &Path, value: &T) -> Result<(), PersistError> {
    let fresh = serialize(path, value)?
        .parse::<DocumentMut>()
        .expect("serialised TOML is valid TOML");

    let mut document = match read(path)? {
        None => DocumentMut::new(),
        Some(text) => match text.parse::<DocumentMut>() {
            Ok(document) => document,
            Err(_) => {
                let backup = backup_path(path);
                publish_atomic(&backup, &text)
                    .map_err(|source| PersistError::io(&backup, source))?;
                DocumentMut::new()
            }
        },
    };

    sync_table(document.as_table_mut(), fresh.as_table());
    write_atomic(path, &document.to_string()).map_err(|source| PersistError::io(path, source))
}

fn read(path: &Path) -> Result<Option<String>, PersistError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(PersistError::io(path, source)),
    }
}

fn serialize<T: Serialize>(path: &Path, value: &T) -> Result<String, PersistError> {
    toml::to_string_pretty(value).map_err(|source| PersistError::Serialize {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".bak");
    path.with_file_name(name)
}

/// Makes `target` hold exactly the keys and values of `source` while leaving
/// the formatting of everything that did not change alone.
fn sync_table(target: &mut Table, source: &Table) {
    let stale: Vec<String> = target
        .iter()
        .map(|(key, _)| key.to_owned())
        .filter(|key| !source.contains_key(key))
        .collect();
    for key in stale {
        target.remove(&key);
    }

    for (key, item) in source.iter() {
        // `Table::insert` reformats an existing key, which would drop the
        // comment above it; assign through the existing item instead.
        match target.get_mut(key) {
            Some(existing) => sync_item(existing, item),
            None => {
                target.insert(key, item.clone());
            }
        }
    }
}

fn sync_item(existing: &mut Item, item: &Item) {
    if let (Some(existing), Some(table)) = (existing.as_table_mut(), item.as_table()) {
        sync_table(existing, table);
        return;
    }
    if let (Some(existing), Some(value)) = (existing.as_value_mut(), item.as_value()) {
        // The decor carries the trailing comment (`size = 13 # px`).
        let decor = existing.decor().clone();
        *existing = value.clone();
        *existing.decor_mut() = decor;
        return;
    }
    *existing = item.clone();
}

fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    // Write through a symlink rather than replacing it: configuration is
    // often linked in from a dotfiles repository.
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    publish_atomic(&path, contents)
}

// Backups publish at their exact path: an old .bak symlink must be replaced,
// never followed into an unrelated file.
fn publish_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(directory)?;

    // Unique, exclusively created temporary files cannot follow an attacker-
    // supplied name or collide with another writer in the same process.
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(contents.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    // Publication has committed. A directory-sync failure must not be reported
    // as a precommit failure: callers would retain state different from disk.
    #[cfg(unix)]
    if let Ok(directory) = fs::File::open(directory) {
        let _ = directory.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Sample {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        size: Option<u32>,
        nested: Nested,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Nested {
        enabled: bool,
    }

    fn sample() -> Sample {
        Sample {
            name: "a".into(),
            size: Some(13),
            nested: Nested { enabled: true },
        }
    }

    #[test]
    fn missing_file_loads_as_none() {
        let directory = tempfile::tempdir().unwrap();
        let loaded: Option<Sample> = load(&directory.path().join("absent.toml")).unwrap();
        assert_eq!(loaded, None);
    }

    #[test]
    fn save_round_trips_and_creates_parent_directories() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deep/er/sample.toml");

        save(&path, &sample()).unwrap();

        assert_eq!(load::<Sample>(&path).unwrap(), Some(sample()));
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, ["sample.toml"]);
    }

    #[test]
    fn invalid_file_reports_its_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("broken.toml");
        fs::write(&path, "name = ").unwrap();

        let error = load::<Sample>(&path).unwrap_err();

        assert!(error.to_string().contains("broken.toml"), "{error}");
    }

    #[test]
    fn preserving_save_keeps_comments_and_order() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.toml");
        fs::write(
            &path,
            "# Who this is.\nname = \"old\"\nsize = 1\n\n[nested]\n# Toggle.\nenabled = false # for now\n",
        )
        .unwrap();

        save_preserving(&path, &sample()).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# Who this is.\nname = \"a\""), "{text}");
        assert!(
            text.contains("# Toggle.\nenabled = true # for now"),
            "{text}"
        );
        assert_eq!(load::<Sample>(&path).unwrap(), Some(sample()));
    }

    #[test]
    fn preserving_save_removes_keys_that_became_unset() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.toml");
        save_preserving(&path, &sample()).unwrap();

        let unset = Sample {
            size: None,
            ..sample()
        };
        save_preserving(&path, &unset).unwrap();

        assert!(!fs::read_to_string(&path).unwrap().contains("size"));
        assert_eq!(load::<Sample>(&path).unwrap(), Some(unset));
    }

    #[test]
    fn preserving_save_backs_up_an_unparsable_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.toml");
        fs::write(&path, "name = = =").unwrap();

        save_preserving(&path, &sample()).unwrap();

        assert_eq!(
            fs::read_to_string(directory.path().join("sample.toml.bak")).unwrap(),
            "name = = ="
        );
        assert_eq!(load::<Sample>(&path).unwrap(), Some(sample()));
    }

    #[cfg(unix)]
    #[test]
    fn invalid_configuration_backup_cannot_follow_an_existing_symlink() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.toml");
        let backup = directory.path().join("sample.toml.bak");
        let victim = directory.path().join("victim");
        fs::write(&path, "name = = =").unwrap();
        fs::write(&victim, "preserve unrelated data").unwrap();
        std::os::unix::fs::symlink(&victim, &backup).unwrap();
        save_preserving(&path, &sample()).unwrap();
        assert_eq!(
            fs::read_to_string(&victim).unwrap(),
            "preserve unrelated data"
        );
        assert_eq!(fs::read_to_string(&backup).unwrap(), "name = = =");
        assert!(
            !fs::symlink_metadata(backup)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[test]
    fn predictable_temporary_symlink_cannot_overwrite_another_file() {
        let directory = tempfile::tempdir().unwrap();
        let victim = directory.path().join("victim");
        fs::write(&victim, b"preserve unrelated data").unwrap();
        let path = directory.path().join("sample.toml");
        let old_temporary = directory
            .path()
            .join(format!(".sample.toml.{}.tmp", std::process::id()));
        std::os::unix::fs::symlink(&victim, &old_temporary).unwrap();
        save(&path, &sample()).unwrap();
        assert_eq!(fs::read(&victim).unwrap(), b"preserve unrelated data");
        assert!(
            fs::symlink_metadata(old_temporary)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn simultaneous_writers_publish_complete_documents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.toml");
        std::thread::scope(|scope| {
            for number in 0..16 {
                let path = &path;
                scope.spawn(move || {
                    for _ in 0..10 {
                        let mut value = sample();
                        value.name = format!("writer-{number}");
                        save(path, &value).unwrap();
                        assert!(load::<Sample>(path).unwrap().is_some());
                    }
                });
            }
        });
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn save_writes_through_a_symlink() {
        let directory = tempfile::tempdir().unwrap();
        let real = directory.path().join("real.toml");
        let link = directory.path().join("link.toml");
        fs::write(&real, "").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        save(&link, &sample()).unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(load::<Sample>(&real).unwrap(), Some(sample()));
    }
}

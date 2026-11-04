//! Cooperative document publication and optimistic conflict detection.
//! Lock files are stable siblings of the resolved configuration target and are never unlinked.
use super::{PersistError, read};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};
use toml_edit::DocumentMut;

/// One document owner and the revision it last read or successfully published.
/// Clones share that revision; independently constructed owners detect stale edits.
///
/// Native callers dispatch load/save on their disk worker. A non-cooperating editor can
/// still race the final rename; the lock coordinates Nocterm writers, not arbitrary programs.
#[derive(Clone, Debug)]
pub struct RevisionedFile {
    path: PathBuf,
    observed: Arc<Mutex<Option<Revision>>>,
}

#[derive(Debug, PartialEq, Eq)]
struct Revision {
    target: PathBuf,
    content: Content,
}
#[derive(Debug, PartialEq, Eq)]
enum Content {
    Missing,
    Tables(BTreeMap<String, [u8; 32]>),
    Invalid([u8; 32]),
}

impl RevisionedFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            observed: Arc::default(),
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reading observes the same bytes used to decode the returned document.
    pub fn load<T: DeserializeOwned>(&self) -> Result<Option<T>, PersistError> {
        let mut observed = self.observed.lock().unwrap_or_else(PoisonError::into_inner);
        let target = resolve(&self.path).map_err(|error| PersistError::io(&self.path, error))?;
        let text = read(&target)?;
        let parsed = text.as_deref().map(str::parse::<DocumentMut>);
        *observed = Some(revision(target, text.as_deref(), parsed.as_ref()));
        text.map(|text| {
            toml::from_str(&text).map_err(|source| PersistError::Parse {
                path: self.path.clone(),
                source: Box::new(source),
            })
        })
        .transpose()
    }

    pub fn save_preserving<T: Serialize>(&self, value: &T) -> Result<(), PersistError> {
        self.save_preserving_owned(value, |_| true)
    }

    /// Unowned top-level tables may change independently and keep their current bytes.
    /// Conflicts in owned tables leave the document and observed revision untouched.
    pub fn save_preserving_owned<T: Serialize>(
        &self,
        value: &T,
        owns: impl Fn(&str) -> bool,
    ) -> Result<(), PersistError> {
        let mut observed = self.observed.lock().unwrap_or_else(PoisonError::into_inner);
        let target = resolve(&self.path).map_err(|error| PersistError::io(&self.path, error))?;
        let parent = target.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|error| PersistError::io(parent, error))?;
        let _lock = publication_lock(&target).map_err(|error| PersistError::io(&target, error))?;
        let text = read(&target)?;
        let parsed = text.as_deref().map(str::parse::<DocumentMut>);
        let current = revision(target.clone(), text.as_deref(), parsed.as_ref());
        if observed
            .as_ref()
            .is_some_and(|prior| conflicts(prior, &current, &owns))
        {
            return Err(PersistError::Conflict {
                path: self.path.clone(),
            });
        }
        let mut document = match parsed {
            Some(Ok(document)) => document,
            // Preserve the existing malformed-file backup contract after conflict admission.
            _ => super::formatting_document(&target, text.as_deref())?,
        };
        super::merge_document(&target, value, &owns, text.is_some(), &mut document)?;
        let published = document.to_string();
        super::write_atomic(&target, &published)
            .map_err(|error| PersistError::io(&target, error))?;
        *observed = Some(Revision {
            target,
            content: tables(&document),
        });
        Ok(())
    }
}

fn resolve(path: &Path) -> io::Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let absolute = std::path::absolute(path)?;
            let parent = absolute.parent().unwrap_or(Path::new("."));
            Ok(fs::canonicalize(parent).map_or(absolute.clone(), |parent| {
                parent.join(absolute.file_name().unwrap_or_default())
            }))
        }
        Err(error) => Err(error),
    }
}

fn publication_lock(target: &Path) -> io::Result<File> {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    let path = target.with_file_name(name);
    let file = match crate::fs::create_private(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => open_existing_lock(&path)?,
        Err(error) => return Err(error.into_io_error()),
    };
    file.try_lock().map_err(io::Error::other)?;
    Ok(file)
}

#[cfg(unix)]
fn open_existing_lock(path: &Path) -> io::Result<File> {
    crate::fs::open_private_regular(path).map_err(io::Error::other)
}
#[cfg(not(unix))]
fn open_existing_lock(path: &Path) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        // Open the reparse point itself; it must not select another file's lock.
        options.custom_flags(0x0020_0000);
    }
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    Ok(file)
}

fn revision(
    target: PathBuf,
    text: Option<&str>,
    parsed: Option<&Result<DocumentMut, toml_edit::TomlError>>,
) -> Revision {
    let content = match (text, parsed) {
        (_, Some(Ok(document))) => tables(document),
        (Some(text), _) => Content::Invalid(digest(text)),
        _ => Content::Missing,
    };
    Revision { target, content }
}
fn digest(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}
fn tables(document: &DocumentMut) -> Content {
    Content::Tables(
        document
            .iter()
            .map(|(key, item)| (key.to_owned(), digest(&item.to_string())))
            .collect(),
    )
}
fn conflicts(prior: &Revision, current: &Revision, owns: &impl Fn(&str) -> bool) -> bool {
    if prior.target != current.target {
        return true;
    }
    match (&prior.content, &current.content) {
        (Content::Tables(prior), Content::Tables(current)) => prior
            .keys()
            .chain(current.keys())
            .any(|key| owns(key) && prior.get(key) != current.get(key)),
        // The empty document and a missing file are equivalent for owned keys.
        (Content::Missing, Content::Tables(current)) => current.keys().any(|key| owns(key)),
        (Content::Tables(prior), Content::Missing) => prior.keys().any(|key| owns(key)),
        (prior, current) => prior != current,
    }
}

#[cfg(test)]
mod tests;

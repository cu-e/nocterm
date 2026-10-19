//! Agent-state TOML files with private atomic publication.
use nocterm_ai::{
    favorites::AgentStateFile,
    storage::{AgentStateRepository, AgentStorageError, StorageFuture},
};
use nocterm_core::SharedError;
use std::path::{Path, PathBuf};
#[derive(Clone)]
pub struct FileAgentStateRepository {
    path: PathBuf,
}
impl FileAgentStateRepository {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}
impl AgentStateRepository for FileAgentStateRepository {
    fn load(&self) -> StorageFuture<AgentStateFile> {
        let path = self.path.clone();
        Box::pin(async move { load(&path) })
    }
    fn save(&self, state: AgentStateFile) -> StorageFuture<()> {
        let path = self.path.clone();
        Box::pin(async move { save(&path, &state) })
    }
}
fn load(path: &Path) -> Result<AgentStateFile, AgentStorageError> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text)
            .map_err(|source| AgentStorageError::StateDecode(SharedError::from_error(source))),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(AgentStateFile::default())
        }
        Err(source) => Err(AgentStorageError::Read {
            path: path.to_owned(),
            source: SharedError::from_error(source),
        }),
    }
}
fn save(path: &Path, state: &AgentStateFile) -> Result<(), AgentStorageError> {
    if let Some(parent) = path.parent() {
        nocterm_core::paths::ensure_private_dir(parent).map_err(|source| {
            AgentStorageError::Directory {
                path: parent.to_owned(),
                source: SharedError::from_error(source),
            }
        })?;
    }
    nocterm_core::persist::save(path, state).map_err(|source| AgentStorageError::StatePublication {
        path: path.to_owned(),
        source: SharedError::from_error(source),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    fn concrete<'a, E: Error + 'static>(error: &'a (dyn Error + 'static)) -> Option<&'a E> {
        std::iter::successors(Some(error), |&error| error.source())
            .find_map(|error| error.downcast_ref())
    }
    /// The original error is one source frame below the variant and survives clones.
    fn assert_retained<E: Error + 'static>(error: &AgentStorageError, held: &E) {
        let found = concrete::<E>(error).expect("original concrete error");
        assert!(std::ptr::eq(found, held));
        let clone = error.clone();
        assert!(std::ptr::eq(concrete::<E>(&clone).unwrap(), held));
        assert_eq!(clone.to_string(), error.to_string());
    }
    #[test]
    fn missing_state_defaults_but_malformed_state_retains_its_typed_source_and_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agents.toml");
        assert_eq!(load(&path).unwrap(), AgentStateFile::default());
        assert!(!path.exists());
        std::fs::write(&path, b"not = [toml").unwrap();
        let before = std::fs::read(&path).unwrap();
        let error = load(&path).unwrap_err();
        let AgentStorageError::StateDecode(held) = &error else {
            panic!("malformed state is StateDecode: {error:?}");
        };
        assert_retained(&error, held.as_ref());
        assert!(held.as_ref().span().is_some());
        assert_eq!(
            error.to_string(),
            format!("Could not decode agent state: {}", held.as_ref())
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    #[test]
    fn state_port_round_trips_latest_preferences_with_private_publication() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("private");
        let path = directory.join("agents.toml");
        let repository = FileAgentStateRepository::new(path.clone());
        let mut state = AgentStateFile::default();
        state.toggle("codex", "model", "one");
        state.toggle("codex", "model", "two");
        state.toggle("codex", "model", "one");
        futures::executor::block_on(repository.save(state.clone())).unwrap();
        assert_eq!(
            futures::executor::block_on(repository.load()).unwrap(),
            state
        );
        assert_eq!(
            std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    #[test]
    fn failed_state_publication_preserves_the_core_error_source() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agents.toml");
        std::fs::create_dir(&path).unwrap();
        let error = save(&path, &AgentStateFile::default()).unwrap_err();
        let AgentStorageError::StatePublication {
            path: reported,
            source,
        } = &error
        else {
            panic!("a blocked state file is StatePublication: {error:?}");
        };
        assert_eq!(reported, &path);
        assert_retained(&error, source.as_ref());
        let native =
            concrete::<std::io::Error>(&error).expect("native cause below the persist error");
        assert!(
            concrete::<std::io::Error>(source.as_ref()).is_some_and(|io| std::ptr::eq(io, native))
        );
        assert_eq!(
            error.to_string(),
            format!(
                "Could not save agent state at {}: {}",
                path.display(),
                source.as_ref()
            )
        );
        assert!(path.is_dir());
    }
    #[test]
    fn unreadable_state_reports_its_native_read_error_instead_of_defaults() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agents.toml");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("marker"), b"kept").unwrap();
        let error = load(&path).unwrap_err();
        let AgentStorageError::Read {
            path: reported,
            source,
        } = &error
        else {
            panic!("a directory at the state path is a Read failure: {error:?}");
        };
        assert_eq!(reported, &path);
        assert_retained(&error, source.as_ref());
        assert_ne!(source.as_ref().kind(), std::io::ErrorKind::NotFound);
        assert_eq!(
            error.to_string(),
            format!("Could not read {}: {}", path.display(), source.as_ref())
        );
        assert_eq!(std::fs::read(path.join("marker")).unwrap(), b"kept");
    }
    #[test]
    fn a_file_blocking_the_state_directory_reports_its_native_directory_error() {
        let root = tempfile::tempdir().unwrap();
        let blocker = root.path().join("agent");
        std::fs::write(&blocker, b"blocker").unwrap();
        let path = blocker.join("agents.toml");
        let error = save(&path, &AgentStateFile::default()).unwrap_err();
        let AgentStorageError::Directory {
            path: reported,
            source,
        } = &error
        else {
            panic!("a blocked parent is a Directory failure: {error:?}");
        };
        assert_eq!(reported, &blocker);
        assert_retained(&error, source.as_ref());
        assert_eq!(
            error.to_string(),
            format!(
                "Could not create private directory {}: {}",
                blocker.display(),
                source.as_ref()
            )
        );
        assert_eq!(std::fs::read(&blocker).unwrap(), b"blocker");
    }
}

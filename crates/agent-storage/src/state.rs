//! Agent-state TOML files with private atomic publication.
use nocterm_ai::{
    favorites::AgentStateFile,
    storage::{AgentStateRepository, AgentStorageError, StorageFuture},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
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
        Ok(text) => {
            toml::from_str(&text).map_err(|source| AgentStorageError::StateDecode(Arc::new(source)))
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(AgentStateFile::default())
        }
        Err(source) => Err(AgentStorageError::Read {
            path: path.to_owned(),
            source: Arc::new(source),
        }),
    }
}
fn save(path: &Path, state: &AgentStateFile) -> Result<(), AgentStorageError> {
    if let Some(parent) = path.parent() {
        nocterm_core::paths::ensure_private_dir(parent).map_err(|source| {
            AgentStorageError::Directory {
                path: parent.to_owned(),
                source: Arc::new(source),
            }
        })?;
    }
    nocterm_core::persist::save(path, state).map_err(|source| AgentStorageError::StatePublication {
        path: path.to_owned(),
        source: Arc::new(source),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;
    #[test]
    fn missing_state_defaults_but_malformed_state_retains_its_typed_source_and_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agents.toml");
        assert_eq!(load(&path).unwrap(), AgentStateFile::default());
        assert!(!path.exists());
        std::fs::write(&path, b"not = [toml").unwrap();
        let before = std::fs::read(&path).unwrap();
        let error = load(&path).unwrap_err();
        assert!(matches!(error, AgentStorageError::StateDecode(_)));
        assert!(error.source().is_some());
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
        assert!(matches!(error, AgentStorageError::StatePublication { .. }));
        assert!(error.source().and_then(|source| source.source()).is_some());
        assert!(path.is_dir());
    }
}

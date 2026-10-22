//! Context paths are selected once, then generated descendants use held operations.
use nocterm_ai::{history::ContextId, storage::AgentStorageError};
use nocterm_session::local_fs::{LocalDirectory, LocalFs, LocalFsError};
use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
    sync::Arc,
};
pub(super) fn validate_id(id: &str) -> Result<(), AgentStorageError> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(AgentStorageError::InvalidId(id.into()));
    }
    Ok(())
}

#[derive(Clone)]
pub(super) struct Root {
    path: PathBuf,
    filesystem: Arc<dyn LocalFs>,
}
impl Root {
    pub(super) fn new(path: PathBuf, filesystem: Arc<dyn LocalFs>) -> Self {
        Self { path, filesystem }
    }
    pub(super) fn open(
        &self,
        create: bool,
    ) -> Result<Option<Arc<dyn LocalDirectory>>, AgentStorageError> {
        if !create {
            match std::fs::symlink_metadata(&self.path) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(source) => {
                    return Err(AgentStorageError::Read {
                        path: self.path.clone(),
                        source: Arc::new(source),
                    });
                }
            }
        }
        nocterm_core::paths::ensure_private_dir(&self.path).map_err(|source| {
            AgentStorageError::Directory {
                path: self.path.clone(),
                source: Arc::new(source),
            }
        })?;
        // Only the configured private root's ancestry is trusted. Resolving it
        // preserves supported aliases there; no generated descendant uses a path.
        let root = match std::fs::canonicalize(&self.path) {
            Ok(root) => root,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(AgentStorageError::Read {
                    path: self.path.clone(),
                    source: Arc::new(source),
                });
            }
        };
        Ok(Some(self.filesystem.open_directory(&root)?))
    }
}

#[derive(Clone)]
pub(super) struct Partition {
    directory: Arc<dyn LocalDirectory>,
    scope: ContextId,
    legacy: bool,
}
impl Partition {
    pub(super) fn legacy(directory: Arc<dyn LocalDirectory>) -> Self {
        Self {
            directory,
            scope: ContextId::General,
            legacy: true,
        }
    }
    pub(super) fn scoped(
        root: &Arc<dyn LocalDirectory>,
        scope: ContextId,
        create: bool,
    ) -> Result<Option<Self>, AgentStorageError> {
        if scope != ContextId::General && !root.identity().is_held() {
            return Err(AgentStorageError::ScopeUnsupported(scope));
        }
        let mut directory = root.clone();
        let names = match scope {
            ContextId::General => vec![OsString::from("scoped-v2"), OsString::from("general")],
            ContextId::Project(id) => {
                vec!["scoped-v2".into(), "projects".into(), id.to_string().into()]
            }
        };
        for name in names {
            directory = match if create {
                directory.create_child(&name)
            } else {
                directory.child(&name)
            } {
                Ok(directory) => directory,
                Err(LocalFsError::NotFound(_)) => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            if scope != ContextId::General && !directory.identity().is_held() {
                return Err(AgentStorageError::ScopeUnsupported(scope));
            }
        }
        Ok(Some(Self {
            directory,
            scope,
            legacy: false,
        }))
    }
    pub(super) fn scope(&self) -> ContextId {
        self.scope
    }
    pub(super) fn is_legacy(&self) -> bool {
        self.legacy
    }
    pub(super) fn directory(&self) -> &Arc<dyn LocalDirectory> {
        &self.directory
    }
    pub(super) fn name(&self, id: &str, extension: &str) -> Result<OsString, AgentStorageError> {
        validate_id(id)?;
        Ok(format!("{id}.{extension}").into())
    }
    pub(super) fn exists(&self, id: &str) -> Result<bool, AgentStorageError> {
        Ok(self.directory.metadata(&self.name(id, "json")?)?.is_some())
    }
    pub(super) fn remove(&self, id: &str, extension: &str) -> Result<(), AgentStorageError> {
        match self.directory.remove_file(&self.name(id, extension)?) {
            Ok(()) | Err(LocalFsError::NotFound(_)) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
    pub(super) fn id(name: &OsStr) -> Option<&str> {
        let name = name.to_str()?.strip_suffix(".json")?;
        uuid::Uuid::parse_str(name).ok()?;
        Some(name)
    }
}

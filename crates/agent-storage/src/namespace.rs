//! Context paths are selected once, then generated descendants use held operations.
use nocterm_ai::{history::ContextId, local_environment::ChatScope, storage::AgentStorageError};
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
    scope: ChatScope,
    version: u32,
}
impl Partition {
    pub(super) fn legacy(directory: Arc<dyn LocalDirectory>) -> Self {
        Self {
            directory,
            scope: ContextId::General.into(),
            version: 1,
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
        let names = context_names(scope);
        let names = std::iter::once(OsString::from("scoped-v2"))
            .chain(names)
            .collect();
        open_partition(
            root,
            scope.into(),
            create,
            names,
            2,
            scope != ContextId::General,
        )
    }
    pub(super) fn environment(
        root: &Arc<dyn LocalDirectory>,
        scope: &ChatScope,
        create: bool,
    ) -> Result<Option<Self>, AgentStorageError> {
        if !root.identity().is_held() && !create && scope.environment().is_this_computer() {
            return Ok(None);
        }
        let mut names = vec![
            OsString::from("scoped-v3"),
            scope.environment().storage_key().into(),
        ];
        names.extend(context_names(scope.context()));
        open_partition(root, scope.clone(), create, names, 3, true)
    }
    pub(super) fn chat_scope(&self) -> &ChatScope {
        &self.scope
    }
    pub(super) fn version(&self) -> u32 {
        self.version
    }
    pub(super) fn is_legacy(&self) -> bool {
        self.version == 1
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

fn context_names(scope: ContextId) -> Vec<OsString> {
    match scope {
        ContextId::General => vec!["general".into()],
        ContextId::Project(id) => vec!["projects".into(), id.to_string().into()],
    }
}
fn open_partition(
    root: &Arc<dyn LocalDirectory>,
    scope: ChatScope,
    create: bool,
    names: Vec<OsString>,
    version: u32,
    held: bool,
) -> Result<Option<Partition>, AgentStorageError> {
    let mut directory = root.clone();
    if held && !directory.identity().is_held() {
        return Err(AgentStorageError::EnvironmentUnsupported(Arc::new(
            scope.environment().clone(),
        )));
    }
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
        if held && !directory.identity().is_held() {
            return Err(AgentStorageError::EnvironmentUnsupported(Arc::new(
                scope.environment().clone(),
            )));
        }
    }
    Ok(Some(Partition {
        directory,
        scope,
        version,
    }))
}

pub(super) fn readable(
    root: &Arc<dyn LocalDirectory>,
    scope: &ChatScope,
) -> Result<Vec<Partition>, AgentStorageError> {
    let mut partitions = Vec::new();
    if let Some(current) = Partition::environment(root, scope, false)? {
        partitions.push(current);
    }
    if scope.environment().is_this_computer() {
        if let Some(previous) = Partition::scoped(root, scope.context(), false)? {
            partitions.push(previous);
        }
        if scope.context() == ContextId::General {
            partitions.push(Partition::legacy(root.clone()));
        }
    }
    Ok(partitions)
}

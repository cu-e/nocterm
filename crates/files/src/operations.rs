//! Pinned file targets and asynchronous operations shared by both Explorer halves.
use super::local_operations;
use nocterm_session::{
    EntryKind, FileMetadata, FsCapabilities, FsError, FsFuture, RemoteFs, Target, fs::path,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone)]
pub(super) enum FileTarget {
    Local(PathBuf),
    Remote {
        path: String,
        host: Target,
        fs: Arc<dyn RemoteFs>,
    },
}
impl FileTarget {
    pub(super) fn name(&self) -> String {
        match self {
            Self::Local(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Self::Remote { path, .. } => path::file_name(path).into(),
        }
    }
    pub(super) fn path(&self) -> String {
        match self {
            Self::Local(p) => p.to_string_lossy().into_owned(),
            Self::Remote { path, .. } => path.clone(),
        }
    }
    pub(super) fn location(&self) -> String {
        match self {
            Self::Local(_) => "Local computer".into(),
            Self::Remote { host, .. } => host.to_string(),
        }
    }
    pub(super) fn capabilities(&self) -> FsCapabilities {
        match self {
            Self::Local(_) => FsCapabilities {
                metadata: true,
                rename: cfg!(any(
                    target_os = "linux",
                    target_os = "android",
                    target_os = "macos",
                    target_os = "ios",
                    windows
                )),
                remove: true,
                set_permissions: cfg!(unix),
            },
            Self::Remote { fs, .. } => fs.capabilities(),
        }
    }
    // This future is polled on the background executor, including local I/O.
    pub(super) fn metadata(&self) -> FsFuture<FileMetadata> {
        let target = self.clone();
        Box::pin(async move {
            match target {
                Self::Local(p) => local_operations::metadata(&p),
                Self::Remote { path, fs, .. } => {
                    remote_path(&path)?;
                    fs.metadata(&path).await
                }
            }
        })
    }
    pub(super) fn rename(&self, name: String) -> FsFuture<()> {
        let target = self.clone();
        Box::pin(async move {
            if !local_operations::valid_name(&name) {
                return Err(FsError::Other(
                    "Enter a single nonempty name without separators.".into(),
                ));
            }
            if target.name() == name {
                return Ok(());
            }
            match target {
                Self::Local(p) => local_operations::rename(&p, &name),
                Self::Remote { path, fs, .. } => {
                    remote_path(&path)?;
                    let destination = path::join(path::parent(&path), &name);
                    fs.rename(&path, &destination).await
                }
            }
        })
    }
    pub(super) fn set_permissions(&self, mode: u32) -> FsFuture<()> {
        let target = self.clone();
        Box::pin(async move {
            match target {
                Self::Local(p) => local_operations::permissions(&p, mode),
                Self::Remote { path, fs, .. } => {
                    remote_path(&path)?;
                    fs.set_permissions(&path, mode).await
                }
            }
        })
    }
    pub(super) fn remove(&self, cancel: Arc<AtomicBool>) -> FsFuture<()> {
        let target = self.clone();
        Box::pin(async move {
            match target {
                Self::Local(p) => local_operations::remove(&p, &cancel),
                Self::Remote { path, fs, .. } => {
                    remote_path(&path)?;
                    remove_remote(fs, path, cancel).await
                }
            }
        })
    }
}
fn remote_path(path: &str) -> Result<(), FsError> {
    if !path.starts_with('/')
        || path == "/"
        || path.contains('\0')
        || path.split('/').any(|p| p == "." || p == "..")
    {
        Err(FsError::Other(
            "Unsafe remote path; the filesystem root cannot be changed.".into(),
        ))
    } else {
        Ok(())
    }
}
async fn remove_remote(
    fs: Arc<dyn RemoteFs>,
    root: String,
    cancel: Arc<AtomicBool>,
) -> Result<(), FsError> {
    // Post-order traversal holds only directory listings on the current branch.
    // Every node is lstat-ed again; final links are unlinked, never traversed.
    const MAX_PENDING_PATH_BYTES: usize = 16 * 1024 * 1024;
    let mut pending_path_bytes = root.len();
    let mut pending = vec![(root, false, 0usize)];
    let mut visited = 0usize;
    while let Some((path, remove_directory, depth)) = pending.pop() {
        pending_path_bytes -= path.len();
        if cancel.load(Ordering::Acquire) {
            return Err(FsError::Other(
                "Deletion cancelled; already deleted entries cannot be restored.".into(),
            ));
        }
        if depth > 256 || visited > 1_000_000 {
            return Err(FsError::Other(
                "Deletion limit reached; refresh to see remaining entries.".into(),
            ));
        }
        if remove_directory {
            fs.remove_dir(&path).await?;
            continue;
        }
        visited += 1;
        let metadata = fs.metadata(&path).await?;
        if metadata.kind == EntryKind::Directory && !metadata.is_symlink {
            let entries = fs.read_dir(&path).await?;
            if visited
                .saturating_add(pending.len())
                .saturating_add(entries.len())
                > 1_000_000
            {
                return Err(FsError::Other(
                    "Deletion discovery limit reached; refresh to see remaining entries.".into(),
                ));
            }
            pending_path_bytes = pending_path_bytes.saturating_add(path.len());
            if pending_path_bytes > MAX_PENDING_PATH_BYTES {
                return Err(FsError::Other("Deletion pending paths exceed 16 MiB; select a smaller subtree and refresh to see remaining entries.".into()));
            }
            pending.push((path.clone(), true, depth));
            for entry in entries.into_iter().rev() {
                if !local_operations::valid_name(&entry.name) {
                    return Err(FsError::Other(
                        "Server returned an unsafe directory entry; deletion stopped.".into(),
                    ));
                }
                let child = path::join(&path, &entry.name);
                pending_path_bytes = pending_path_bytes.saturating_add(child.len());
                if pending_path_bytes > MAX_PENDING_PATH_BYTES {
                    return Err(FsError::Other("Deletion pending paths exceed 16 MiB; select a smaller subtree and refresh to see remaining entries.".into()));
                }
                pending.push((child, false, depth + 1));
            }
        } else {
            fs.remove_file(&path).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_mutations_reject_roots_parent_components_and_nul() {
        for p in ["/", "relative", "/home/../outside", "/home/.", "/home/a\0"] {
            assert!(remote_path(p).is_err());
        }
        assert!(remote_path("/home/unicode-目录").is_ok());
    }
    #[test]
    fn deletion_bounds_pending_path_memory_before_mutating_a_wide_directory() {
        use std::sync::atomic::AtomicUsize;
        struct WideFs(Arc<AtomicUsize>);
        impl RemoteFs for WideFs {
            fn home(&self) -> FsFuture<String> {
                Box::pin(async { Ok("/root".into()) })
            }
            fn metadata(&self, path: &str) -> FsFuture<FileMetadata> {
                let directory = path == "/root";
                Box::pin(async move {
                    Ok(FileMetadata {
                        kind: if directory {
                            EntryKind::Directory
                        } else {
                            EntryKind::File
                        },
                        is_symlink: false,
                        size: None,
                        permissions: None,
                        uid: None,
                        gid: None,
                        modified: None,
                    })
                })
            }
            fn read_dir(&self, _: &str) -> FsFuture<Vec<nocterm_session::DirEntry>> {
                Box::pin(async {
                    Ok(vec![
                        nocterm_session::DirEntry {
                            name: "a".repeat(900),
                            kind: EntryKind::File,
                            is_symlink: false,
                            size: None
                        };
                        20_000
                    ])
                })
            }
            fn remove_file(&self, _: &str) -> FsFuture<()> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(()) })
            }
        }
        let mutations = Arc::new(AtomicUsize::new(0));
        let result = futures::executor::block_on(remove_remote(
            Arc::new(WideFs(mutations.clone())),
            "/root".into(),
            Arc::new(AtomicBool::new(false)),
        ));
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("pending paths exceed 16 MiB")
        );
        assert_eq!(mutations.load(Ordering::SeqCst), 0);
    }
}

use crate::{
    DirEntry, EntryKind, FileMetadata, FsError,
    fs::{UploadMode, path},
};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Component, Path},
};

#[derive(Clone)]
enum Data {
    Directory,
    File(Vec<u8>),
    Link(EntryKind, Option<Key>),
    Other,
}
#[derive(Clone)]
struct Node {
    id: u64,
    data: Data,
    permissions: u32,
    declared_size: Option<Option<u64>>,
}
impl Node {
    fn metadata(&self) -> FileMetadata {
        let (kind, is_symlink, size) = match &self.data {
            Data::Directory => (EntryKind::Directory, false, None),
            Data::File(bytes) => (EntryKind::File, false, Some(bytes.len() as u64)),
            Data::Link(kind, _) => (*kind, true, None),
            Data::Other => (EntryKind::Other, false, None),
        };
        FileMetadata {
            kind,
            is_symlink,
            size: self.declared_size.unwrap_or(size),
            permissions: Some(self.permissions),
            uid: None,
            gid: None,
            modified: None,
        }
    }
}
type Key = Vec<OsString>;
pub(super) struct Tree {
    nodes: BTreeMap<Key, Node>,
    next_id: u64,
}
impl Default for Tree {
    fn default() -> Self {
        let mut tree = Self {
            nodes: BTreeMap::new(),
            next_id: 1,
        };
        tree.insert_root(vec![]);
        #[cfg(windows)]
        tree.insert_root(native_key(Path::new("C:\\")).unwrap_or_default());
        tree
    }
}
impl Tree {
    fn insert_root(&mut self, key: Key) {
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.insert(
            key,
            Node {
                id,
                data: Data::Directory,
                permissions: 0o755,
                declared_size: None,
            },
        );
    }
    fn fresh(&mut self, data: Data) -> Result<Node, FsError> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| FsError::Unsupported("fake inode identifiers exhausted".into()))?;
        Ok(Node {
            id,
            data,
            permissions: 0o600,
            declared_size: None,
        })
    }
    fn parents(&self, key: &[OsString], label: &str) -> Result<(), FsError> {
        for count in 0..key.len() {
            let ancestor = &key[..count];
            // Native Windows roots are namespace values, not user filenames.
            if count == 0
                && key
                    .first()
                    .is_some_and(|part| part.as_encoded_bytes().contains(&0))
            {
                continue;
            }
            match self.nodes.get(ancestor).map(|node| &node.data) {
                Some(Data::Directory) => {}
                Some(_) => return Err(denied(label)),
                None => return Err(missing(label)),
            }
        }
        Ok(())
    }
    fn node(&self, key: &[OsString], label: &str) -> Result<&Node, FsError> {
        self.parents(key, label)?;
        self.nodes.get(key).ok_or_else(|| missing(label))
    }
    fn list_key(
        &self,
        key: &[OsString],
        label: &str,
    ) -> Result<Vec<(OsString, FileMetadata)>, FsError> {
        if !matches!(self.node(key, label)?.data, Data::Directory) {
            return Err(denied(label));
        }
        Ok(self
            .nodes
            .iter()
            .filter(|(child, _)| child.len() == key.len() + 1 && child.starts_with(key))
            .filter_map(|(child, node)| child.last().map(|name| (name.clone(), node.metadata())))
            .collect())
    }
    fn directory(&mut self, key: Key, label: &str) -> Result<(), FsError> {
        self.parents(&key, label)?;
        match self.nodes.get(&key) {
            Some(Node {
                data: Data::Directory,
                ..
            }) => Ok(()),
            Some(_) => Err(FsError::AlreadyExists { path: label.into() }),
            None => {
                let mut node = self.fresh(Data::Directory)?;
                node.permissions = 0o755;
                self.nodes.insert(key, node);
                Ok(())
            }
        }
    }
    fn seed_key(&mut self, key: Key, label: &str, data: Data) -> Result<(), FsError> {
        if key.is_empty() {
            return Err(denied(label));
        }
        for count in 1..key.len() {
            self.directory(key[..count].to_vec(), label)?;
        }
        self.parents(&key, label)?;
        if self
            .nodes
            .get(&key)
            .is_some_and(|node| matches!(node.data, Data::Directory))
        {
            return Err(denied(label));
        }
        let node = self.fresh(data)?;
        self.nodes.insert(key, node);
        Ok(())
    }
    fn bytes_key(&self, key: &[OsString], label: &str) -> Result<Vec<u8>, FsError> {
        match &self.node(key, label)?.data {
            Data::File(bytes) => Ok(bytes.clone()),
            _ => Err(denied(label)),
        }
    }
    fn upload_key(&self, key: &[OsString], label: &str, mode: UploadMode) -> Result<(), FsError> {
        self.parents(key, label)?;
        if let Some(node) = self.nodes.get(key) {
            if mode == UploadMode::Create {
                return Err(FsError::AlreadyExists { path: label.into() });
            }
            if !matches!(node.data, Data::File(_)) {
                return Err(denied(label));
            }
        }
        Ok(())
    }
    fn publish_key(
        &mut self,
        key: Key,
        label: &str,
        mode: UploadMode,
        bytes: Vec<u8>,
    ) -> Result<(), FsError> {
        self.upload_key(&key, label, mode)?;
        let node = self.fresh(Data::File(bytes))?;
        self.nodes.insert(key, node);
        Ok(())
    }
    fn rename_key(&mut self, old: Key, new: Key, label: &str) -> Result<(), FsError> {
        self.node(&old, label)?;
        self.parents(&new, label)?;
        if old.is_empty() || new.starts_with(&old) && new.len() > old.len() {
            return Err(denied(label));
        }
        if self.nodes.contains_key(&new) {
            return Err(FsError::AlreadyExists { path: label.into() });
        }
        let names: Vec<_> = self
            .nodes
            .keys()
            .filter(|key| key.starts_with(&old))
            .cloned()
            .collect();
        for name in names {
            if let Some(node) = self.nodes.remove(&name) {
                let mut moved = new.clone();
                moved.extend_from_slice(&name[old.len()..]);
                self.nodes.insert(moved, node);
            }
        }
        Ok(())
    }
    fn remove_key(&mut self, key: Key, label: &str, directory: bool) -> Result<(), FsError> {
        let node = self.node(&key, label)?;
        if key.is_empty() || matches!(node.data, Data::Directory) != directory {
            return Err(denied(label));
        }
        if directory
            && self
                .nodes
                .keys()
                .any(|child| child.len() > key.len() && child.starts_with(&key))
        {
            return Err(FsError::Unsupported("directory is not empty".into()));
        }
        self.nodes.remove(&key);
        Ok(())
    }
    fn permissions_key(&mut self, key: Key, label: &str, permissions: u32) -> Result<(), FsError> {
        if permissions > 0o7777 {
            return Err(FsError::Unsupported("invalid permission bits".into()));
        }
        if matches!(self.node(&key, label)?.data, Data::Link(_, _)) {
            return Err(denied(label));
        }
        if let Some(node) = self.nodes.get_mut(&key) {
            node.permissions = permissions;
        }
        Ok(())
    }

    pub(super) fn metadata(&self, name: &str) -> Result<FileMetadata, FsError> {
        Ok(self.node(&remote_key(name)?, name)?.metadata())
    }
    pub(super) fn declare_size(&mut self, name: &str, size: Option<u64>) -> Result<(), FsError> {
        let key = remote_key(name)?;
        let node = self.nodes.get_mut(&key).ok_or_else(|| missing(name))?;
        if !matches!(node.data, Data::File(_)) {
            return Err(denied(name));
        }
        node.declared_size = Some(size);
        Ok(())
    }
    pub(super) fn bytes(&self, name: &str) -> Result<Vec<u8>, FsError> {
        self.bytes_key(&remote_key(name)?, name)
    }
    pub(super) fn list(&self, name: &str) -> Result<Vec<DirEntry>, FsError> {
        Ok(self
            .list_key(&remote_key(name)?, name)?
            .into_iter()
            .map(|(name, meta)| entry(&name.to_string_lossy(), meta))
            .collect())
    }
    pub(super) fn create_dir(&mut self, name: &str) -> Result<(), FsError> {
        self.directory(remote_key(name)?, name)
    }
    pub(super) fn seed(
        &mut self,
        name: &str,
        bytes: Option<Vec<u8>>,
        link: Option<EntryKind>,
    ) -> Result<(), FsError> {
        let data = match (bytes, link) {
            (_, Some(kind)) => Data::Link(kind, None),
            (Some(bytes), None) => Data::File(bytes),
            (None, None) => Data::Other,
        };
        self.seed_key(remote_key(name)?, name, data)
    }
    pub(super) fn upload_path(&self, name: &str, mode: UploadMode) -> Result<(), FsError> {
        self.upload_key(&remote_key(name)?, name, mode)
    }
    pub(super) fn publish(
        &mut self,
        name: &str,
        mode: UploadMode,
        bytes: Vec<u8>,
    ) -> Result<(), FsError> {
        self.publish_key(remote_key(name)?, name, mode, bytes)
    }
    pub(super) fn rename(&mut self, old: &str, new: &str) -> Result<(), FsError> {
        self.rename_key(remote_key(old)?, remote_key(new)?, new)
    }
    pub(super) fn remove(&mut self, name: &str, directory: bool) -> Result<(), FsError> {
        self.remove_key(remote_key(name)?, name, directory)
    }
    pub(super) fn permissions(&mut self, name: &str, permissions: u32) -> Result<(), FsError> {
        self.permissions_key(remote_key(name)?, name, permissions)
    }

    pub(super) fn seed_native(&mut self, path: &Path, bytes: Vec<u8>) -> Result<(), FsError> {
        self.seed_key(
            native_key(path)?,
            &path.to_string_lossy(),
            Data::File(bytes),
        )
    }
    pub(super) fn metadata_native(&self, path: &Path) -> Result<FileMetadata, FsError> {
        Ok(self
            .node(&native_key(path)?, &path.to_string_lossy())?
            .metadata())
    }
    pub(super) fn bytes_native(&self, path: &Path) -> Result<Vec<u8>, FsError> {
        self.bytes_key(&native_key(path)?, &path.to_string_lossy())
    }
    pub(super) fn directory_native(&mut self, path: &Path) -> Result<(), FsError> {
        self.directory(native_key(path)?, &path.to_string_lossy())
    }
    pub(super) fn rename_native(&mut self, path: &Path, name: &OsStr) -> Result<(), FsError> {
        let old = native_key(path)?;
        let mut new = old.clone();
        new.pop();
        new.push(name.to_owned());
        self.rename_key(old, new, &path.to_string_lossy())
    }
    pub(super) fn permissions_native(
        &mut self,
        path: &Path,
        permissions: u32,
    ) -> Result<(), FsError> {
        self.permissions_key(native_key(path)?, &path.to_string_lossy(), permissions)
    }
    pub(super) fn seed_native_alias(&mut self, path: &Path, target: &Path) -> Result<(), FsError> {
        let key = native_key(target)?;
        let kind = match self.node(&key, &target.to_string_lossy()) {
            Ok(node) => node.metadata().kind,
            Err(FsError::NotFound { .. }) => EntryKind::Other,
            Err(error) => return Err(error),
        };
        self.seed_key(
            native_key(path)?,
            &path.to_string_lossy(),
            Data::Link(kind, Some(key)),
        )
    }
    pub(super) fn browse_id(&self, path: &Path) -> Result<u64, FsError> {
        if !path.is_absolute() || path.as_os_str().as_encoded_bytes().contains(&0) {
            return Err(FsError::Unsupported("invalid browse path".into()));
        }
        let mut key = Vec::new();
        let mut links = 0;
        for component in path.components() {
            match component {
                Component::RootDir | Component::CurDir => {}
                Component::Prefix(prefix) => {
                    let mut root = OsString::from("\0native-root:");
                    root.push(prefix.as_os_str());
                    key.push(root);
                }
                Component::ParentDir => {
                    if key
                        .last()
                        .is_some_and(|name| !name.as_encoded_bytes().starts_with(b"\0native-root:"))
                    {
                        key.pop();
                    }
                }
                Component::Normal(name) => key.push(name.to_owned()),
            }
            while let Some(Data::Link(_, Some(target))) =
                self.nodes.get(&key).map(|node| &node.data)
            {
                links += 1;
                if links > 64 {
                    return Err(FsError::Unsupported("symlink resolution bound".into()));
                }
                key = target.clone();
            }
            let node = self.node(&key, &path.to_string_lossy())?;
            if !matches!(node.data, Data::Directory) {
                return Err(denied(&path.to_string_lossy()));
            }
        }
        Ok(self.node(&key, &path.to_string_lossy())?.id)
    }
    pub(super) fn directory_id(&self, path: &Path) -> Result<u64, FsError> {
        let key = native_key(path)?;
        let node = self.node(&key, &path.to_string_lossy())?;
        if !matches!(node.data, Data::Directory) {
            return Err(denied(&path.to_string_lossy()));
        }
        Ok(node.id)
    }
    fn held_key(&self, id: u64) -> Result<Key, FsError> {
        self.nodes
            .iter()
            .find(|(_, node)| node.id == id)
            .map(|(key, _)| key.clone())
            .ok_or(FsError::NotConnected)
    }
    pub(super) fn held_list(&self, id: u64) -> Result<Vec<(OsString, FileMetadata)>, FsError> {
        self.list_key(&self.held_key(id)?, "held directory")
    }
    pub(super) fn held_completion_directory(&self, id: u64, name: &OsStr) -> bool {
        let Ok(mut key) = self.held_key(id) else {
            return false;
        };
        key.push(name.to_owned());
        for _ in 0..64 {
            match self.nodes.get(&key).map(|node| &node.data) {
                Some(Data::Directory) => return true,
                Some(Data::Link(_, Some(target))) => key = target.clone(),
                _ => return false,
            }
        }
        false
    }
    pub(super) fn held_metadata(
        &self,
        id: u64,
        name: &OsStr,
    ) -> Result<Option<FileMetadata>, FsError> {
        let mut key = self.held_key(id)?;
        key.push(name.to_owned());
        match self.node(&key, "held directory") {
            Ok(node) => Ok(Some(node.metadata())),
            Err(FsError::NotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }
    pub(super) fn held_bytes(&self, id: u64, name: &OsStr) -> Result<Vec<u8>, FsError> {
        let mut key = self.held_key(id)?;
        key.push(name.to_owned());
        self.bytes_key(&key, "held source")
    }
    pub(super) fn held_child(
        &mut self,
        id: u64,
        name: &OsStr,
        create: bool,
    ) -> Result<u64, FsError> {
        let mut key = self.held_key(id)?;
        key.push(name.to_owned());
        if create {
            self.directory(key.clone(), "held child")?;
        }
        let node = self.node(&key, "held child")?;
        if !matches!(node.data, Data::Directory) {
            return Err(denied("held child"));
        }
        Ok(node.id)
    }
    pub(super) fn held_remove(
        &mut self,
        id: u64,
        name: &OsStr,
        directory: bool,
    ) -> Result<(), FsError> {
        let mut key = self.held_key(id)?;
        key.push(name.to_owned());
        self.remove_key(key, "held removal", directory)
    }
    pub(super) fn held_publish(
        &mut self,
        id: u64,
        name: &OsStr,
        mode: UploadMode,
        bytes: Vec<u8>,
    ) -> Result<(), FsError> {
        let mut key = self.held_key(id)?;
        key.push(name.to_owned());
        self.publish_key(key, "held publication", mode, bytes)
    }
}
fn remote_key(name: &str) -> Result<Key, FsError> {
    if !name.starts_with('/') || name.contains('\0') || (name != "/" && name.ends_with('/')) {
        return Err(FsError::Unsupported("non-canonical absolute path".into()));
    }
    if name == "/" {
        return Ok(vec![]);
    }
    name.split('/')
        .skip(1)
        .map(|part| {
            if part.is_empty() || part == "." || part == ".." {
                Err(FsError::Unsupported("non-canonical absolute path".into()))
            } else {
                Ok(OsString::from(part))
            }
        })
        .collect()
}
fn native_key(path: &Path) -> Result<Key, FsError> {
    if !path.is_absolute() || path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(FsError::Unsupported(
            "non-canonical absolute native path".into(),
        ));
    }
    let mut key = Vec::new();
    for part in path.components() {
        match part {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => key.push(name.to_owned()),
            Component::Prefix(prefix) => {
                let mut root = OsString::from("\0native-root:");
                root.push(prefix.as_os_str());
                key.push(root);
            }
            Component::ParentDir => return Err(FsError::Unsupported("parent component".into())),
        }
    }
    Ok(key)
}
pub(super) fn entry(name: &str, metadata: FileMetadata) -> DirEntry {
    DirEntry {
        name: path::file_name(name).into(),
        kind: metadata.kind,
        is_symlink: metadata.is_symlink,
        size: metadata.size,
    }
}
fn denied(name: &str) -> FsError {
    FsError::PermissionDenied { path: name.into() }
}
fn missing(name: &str) -> FsError {
    FsError::NotFound { path: name.into() }
}

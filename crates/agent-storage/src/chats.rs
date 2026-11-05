//! Immutable scoped documents; the service supplies serialization and write ordering.
use crate::{
    catalog,
    input::Input,
    namespace::{Partition, Root},
    publication,
};
use nocterm_ai::{
    history::{self, CatalogHeader, ChatSummary, ContextId, SavedChat, SharedChat},
    local_environment::ChatScope,
    storage::{
        AgentStorageError, ChatRepository, PreparedChatBytes, PreparedChatLoad, StorageFuture,
        require_local_scope,
    },
};
use nocterm_session::local_fs::{LocalDirectory, LocalFs};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct FileChatRepository {
    root: Root,
}
impl FileChatRepository {
    /// The configured root has trusted application-owned ancestry. Every generated
    /// descendant and operation uses the supplied strict held-directory capability.
    pub fn new(dir: PathBuf, filesystem: Arc<dyn LocalFs>) -> Self {
        Self {
            root: Root::new(dir, filesystem),
        }
    }
}
impl ChatRepository for FileChatRepository {
    fn prepare_load_in(
        &self,
        scope: ChatScope,
        id: String,
    ) -> StorageFuture<Box<dyn PreparedChatLoad>> {
        let root = self.root.clone();
        Box::pin(async move {
            let (bytes, version) = read(&root, &scope, &id)?;
            Ok(
                Box::new(PreparedChatBytes::new(bytes, scope, id, Some(version))?)
                    as Box<dyn PreparedChatLoad>,
            )
        })
    }
    fn catalog(&self, scope: ContextId) -> StorageFuture<Vec<ChatSummary>> {
        self.catalog_in(scope.into())
    }
    fn load(&self, scope: ContextId, id: String) -> StorageFuture<SavedChat> {
        self.load_in(scope.into(), id)
    }
    fn delete(&self, scope: ContextId, id: String) -> StorageFuture<()> {
        self.delete_in(scope.into(), id)
    }
    fn catalog_in(&self, scope: ChatScope) -> StorageFuture<Vec<ChatSummary>> {
        let root = self.root.clone();
        Box::pin(async move { catalog::load(&root, &scope) })
    }
    fn load_in(&self, scope: ChatScope, id: String) -> StorageFuture<SavedChat> {
        let root = self.root.clone();
        Box::pin(async move { load(&root, &scope, &id) })
    }
    fn save(&self, chat: Arc<SharedChat>) -> StorageFuture<()> {
        let root = self.root.clone();
        Box::pin(async move { save(&root, &chat) })
    }
    fn delete_in(&self, scope: ChatScope, id: String) -> StorageFuture<()> {
        let root = self.root.clone();
        Box::pin(async move { delete(&root, &scope, &id) })
    }
}

fn requested(
    root: &Arc<dyn LocalDirectory>,
    scope: &ChatScope,
    id: &str,
) -> Result<Option<Partition>, AgentStorageError> {
    for partition in crate::namespace::readable(root, scope)? {
        if partition.exists(id)? {
            return Ok(Some(partition));
        }
    }
    Ok(None)
}
fn load(root: &Root, scope: &ChatScope, id: &str) -> Result<SavedChat, AgentStorageError> {
    let (bytes, version) = read(root, scope, id)?;
    let chat = history::decode(&bytes, id)?;
    require_local_scope(scope, &chat.header.chat_scope())?;
    if version != chat.version {
        return Err(history::HistoryCodecError::InvalidHeader.into());
    }
    Ok(chat)
}
fn read(root: &Root, scope: &ChatScope, id: &str) -> Result<(Vec<u8>, u32), AgentStorageError> {
    crate::namespace::validate_id(id)?;
    let directory = root
        .open(false)?
        .ok_or_else(|| AgentStorageError::NotFound(id.into()))?;
    let partition =
        requested(&directory, scope, id)?.ok_or_else(|| AgentStorageError::NotFound(id.into()))?;
    let input = Input::open(&partition, id, "json")?;
    let bytes = if partition.is_legacy() {
        input.all()?
    } else {
        input.scoped_record(id, scope)?
    };
    Ok((bytes, partition.version()))
}
fn check_version(partition: &Partition, version: u32) -> Result<(), AgentStorageError> {
    if partition.version() != version {
        return Err(history::HistoryCodecError::InvalidHeader.into());
    }
    Ok(())
}
pub(super) fn header(partition: &Partition, id: &str) -> Result<CatalogHeader, AgentStorageError> {
    let input = Input::open(partition, id, "json")?;
    let header = history::decode_catalog_header(&input.prefix()?, id)?;
    require_local_scope(partition.chat_scope(), &header.chat_scope())?;
    check_version(partition, header.transcript_version().wire())?;
    Ok(header)
}
fn existing(partition: &Partition, id: &str) -> Result<(), AgentStorageError> {
    if !partition.exists(id)? {
        return Ok(());
    }
    if partition.is_legacy() {
        let chat = history::decode(&Input::open(partition, id, "json")?.all()?, id)?;
        require_local_scope(partition.chat_scope(), &chat.header.chat_scope())?;
        check_version(partition, chat.version)
    } else {
        header(partition, id).map(|_| ())
    }
}
fn save(root: &Root, chat: &SharedChat) -> Result<(), AgentStorageError> {
    if let Some((draft, updated)) = chat.metadata.draft_patch() {
        let mut original = load(root, &chat.header.chat_scope(), &chat.header.id)?;
        original.draft = draft;
        original.header.updated = updated;
        return save_record(root, &original, history::encode(&original)?);
    }
    save_record(root, &chat.metadata, history::encode_shared(chat)?)
}
fn save_record(root: &Root, chat: &SavedChat, bytes: Vec<u8>) -> Result<(), AgentStorageError> {
    let directory = root
        .open(true)?
        .ok_or_else(|| AgentStorageError::NotFound(chat.header.id.clone()))?;
    let scope = chat.header.chat_scope();
    let partition = match chat.version {
        1 => {
            require_local_scope(&ContextId::General.into(), &scope)?;
            Some(Partition::legacy(directory.clone()))
        }
        2 => Partition::scoped(&directory, scope.context(), true)?,
        3 => Partition::environment(&directory, &scope, true)?,
        other => return Err(history::HistoryCodecError::UnsupportedVersion(other).into()),
    };
    let partition = partition.ok_or_else(|| AgentStorageError::NotFound(chat.header.id.clone()))?;
    existing(&partition, &chat.header.id)?;
    publication::publish(&partition, &chat.header.id, "json", &bytes)?;
    Ok(())
}
fn delete(root: &Root, scope: &ChatScope, id: &str) -> Result<(), AgentStorageError> {
    crate::namespace::validate_id(id)?;
    let Some(directory) = root.open(false)? else {
        return Ok(());
    };
    let partitions = crate::namespace::readable(&directory, scope)?;
    for partition in &partitions {
        existing(partition, id)?;
    }
    // Oldest first: interruption cannot expose a previously shadowed fallback.
    for partition in partitions.iter().rev() {
        partition.remove(id, "json")?;
        let _ = partition.remove(id, "meta");
    }
    Ok(())
}

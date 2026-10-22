//! Immutable scoped documents; the service supplies serialization and write ordering.
use crate::{
    catalog,
    input::Input,
    namespace::{Partition, Root},
    publication,
};
use nocterm_ai::{
    history::{self, CatalogHeader, ChatSummary, ContextId, SavedChat, SharedChat},
    storage::{AgentStorageError, ChatRepository, StorageFuture, require_chat_scope},
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
    fn catalog(&self, scope: ContextId) -> StorageFuture<Vec<ChatSummary>> {
        let root = self.root.clone();
        Box::pin(async move { catalog::load(&root, scope) })
    }
    fn load(&self, scope: ContextId, id: String) -> StorageFuture<SavedChat> {
        let root = self.root.clone();
        Box::pin(async move { load(&root, scope, &id) })
    }
    fn save(&self, chat: Arc<SharedChat>) -> StorageFuture<()> {
        let root = self.root.clone();
        Box::pin(async move { save(&root, &chat) })
    }
    fn delete(&self, scope: ContextId, id: String) -> StorageFuture<()> {
        let root = self.root.clone();
        Box::pin(async move { delete(&root, scope, &id) })
    }
}
fn requested(
    root: &Arc<dyn LocalDirectory>,
    scope: ContextId,
    id: &str,
) -> Result<Option<Partition>, AgentStorageError> {
    if let Some(partition) = Partition::scoped(root, scope, false)?
        && partition.exists(id)?
    {
        return Ok(Some(partition));
    }
    if scope == ContextId::General {
        let legacy = Partition::legacy(root.clone());
        if legacy.exists(id)? {
            return Ok(Some(legacy));
        }
    }
    Ok(None)
}
fn load(root: &Root, scope: ContextId, id: &str) -> Result<SavedChat, AgentStorageError> {
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
    let chat = history::decode(&bytes, id)?;
    require_chat_scope(scope, chat.header.scope)?;
    check_version(&partition, chat.version)?;
    Ok(chat)
}
fn check_version(partition: &Partition, version: u32) -> Result<(), AgentStorageError> {
    if (partition.is_legacy() && version != 1) || (!partition.is_legacy() && version != 2) {
        return Err(history::HistoryCodecError::InvalidHeader.into());
    }
    Ok(())
}
pub(super) fn header(partition: &Partition, id: &str) -> Result<CatalogHeader, AgentStorageError> {
    let input = Input::open(partition, id, "json")?;
    let header = history::decode_catalog_header(&input.prefix()?, id)?;
    require_chat_scope(partition.scope(), header.scope())?;
    Ok(header)
}
fn existing(partition: &Partition, id: &str) -> Result<(), AgentStorageError> {
    if !partition.exists(id)? {
        return Ok(());
    }
    if partition.is_legacy() {
        let chat = history::decode(&Input::open(partition, id, "json")?.all()?, id)?;
        require_chat_scope(partition.scope(), chat.header.scope)?;
        check_version(partition, chat.version)
    } else {
        header(partition, id).map(|_| ())
    }
}
fn save(root: &Root, chat: &SharedChat) -> Result<(), AgentStorageError> {
    if let Some((draft, updated)) = chat.metadata.draft_patch() {
        let mut original = load(root, chat.header.scope, &chat.header.id)?;
        original.draft = draft;
        original.header.updated = updated;
        return save_record(
            root,
            &original,
            history::encode(&original)?,
            !original.queue.is_empty(),
        );
    }
    save_record(
        root,
        &chat.metadata,
        history::encode_shared(chat)?,
        !chat.queue.is_empty(),
    )
}
fn save_record(
    root: &Root,
    chat: &SavedChat,
    bytes: Vec<u8>,
    queued: bool,
) -> Result<(), AgentStorageError> {
    let directory = root
        .open(true)?
        .ok_or_else(|| AgentStorageError::NotFound(chat.header.id.clone()))?;
    let partition = if chat.version == 1 {
        require_chat_scope(ContextId::General, chat.header.scope)?;
        Partition::legacy(directory.clone())
    } else {
        Partition::scoped(&directory, chat.header.scope, true)?
            .ok_or_else(|| AgentStorageError::NotFound(chat.header.id.clone()))?
    };
    existing(&partition, &chat.header.id)?;
    publication::publish(&partition, &chat.header.id, "json", &bytes)?;
    catalog::update_cache(&partition, chat, queued);
    Ok(())
}
fn delete(root: &Root, scope: ContextId, id: &str) -> Result<(), AgentStorageError> {
    crate::namespace::validate_id(id)?;
    let Some(directory) = root.open(false)? else {
        return Ok(());
    };
    let scoped = Partition::scoped(&directory, scope, false)?;
    // Validate both before mutation. Legacy is removed first so an interrupted
    // delete cannot reveal its stale fallback after removing the scoped document.
    let legacy = (scope == ContextId::General).then(|| Partition::legacy(directory));
    if let Some(legacy) = &legacy {
        existing(legacy, id)?;
    }
    if let Some(scoped) = &scoped {
        existing(scoped, id)?;
    }
    for partition in legacy.into_iter().chain(scoped) {
        partition.remove(id, "json")?;
        let _ = partition.remove(id, "meta");
    }
    Ok(())
}

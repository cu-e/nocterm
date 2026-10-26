//! Per-scope discovery verifies transcript authority before consulting sidecars.
use crate::{
    input::Input,
    namespace::{Partition, Root},
    publication,
};
use nocterm_ai::{
    history::{self, ChatSummary, ContextId, MAX_RESTORED, SavedChat},
    storage::{AgentStorageError, require_chat_scope},
};
const MAX_METADATA_BYTES: usize = 16 * 1024;

pub(super) fn update_cache(partition: &Partition, chat: &SavedChat, queued: bool) {
    let Ok(input) = Input::open(partition, &chat.header.id, "json") else {
        return;
    };
    let summary = ChatSummary::from_chat(chat).with_stamp(input.stamp(), queued);
    write_cache(partition, &summary);
}
fn write_cache(partition: &Partition, summary: &ChatSummary) {
    if let Ok(bytes) = serde_json::to_vec(summary) {
        let _ = publication::publish(partition, &summary.id, "meta", &bytes);
    }
}
fn cached(partition: &Partition, id: &str) -> Option<ChatSummary> {
    let input = Input::open(partition, id, "meta").ok()?;
    if input.stamp().bytes > MAX_METADATA_BYTES as u64 {
        return None;
    }
    serde_json::from_slice(&input.prefix().ok()?).ok()
}
fn summary(partition: &Partition, id: &str) -> Result<Option<ChatSummary>, AgentStorageError> {
    let input = Input::open(partition, id, "json")?;
    let stamp = input.stamp();
    let summary = if partition.is_legacy() {
        // Arbitrary valid v1 field order and fallback titles cannot be proved
        // from a bounded prefix. Keep this compatibility scan; do not rewrite it.
        let bytes = input.all()?;
        if bytes.starts_with(b"{\"version\":2,") {
            return Err(history::HistoryCodecError::InvalidHeader.into());
        }
        history::decode_summary(&bytes, id, stamp)?
    } else {
        let header = history::decode_catalog_header(&input.prefix()?, id)?;
        require_chat_scope(partition.scope(), header.scope())?;
        header.summary(stamp)
    };
    if let Some(summary) = &summary {
        require_chat_scope(partition.scope(), summary.scope)?;
        if cached(partition, id).as_ref() != Some(summary) {
            write_cache(partition, summary);
        }
    }
    Ok(summary)
}
pub(super) fn load(root: &Root, scope: ContextId) -> Result<Vec<ChatSummary>, AgentStorageError> {
    let Some(directory) = root.open(false)? else {
        return Ok(Vec::new());
    };
    let scoped = Partition::scoped(&directory, scope, false)?;
    let mut rows = Vec::new();
    if let Some(partition) = &scoped {
        scan(partition, None, &mut rows)?;
    }
    if scope == ContextId::General {
        scan(&Partition::legacy(directory), scoped.as_ref(), &mut rows)?;
    }
    Ok(rows)
}
fn scan(
    partition: &Partition,
    preferred: Option<&Partition>,
    rows: &mut Vec<ChatSummary>,
) -> Result<(), AgentStorageError> {
    let mut entries = partition.directory().entries()?;
    while let Some(name) = entries.next()? {
        let Some(id) = Partition::id(&name) else {
            continue;
        };
        if let Some(preferred) = preferred
            && preferred.exists(id)?
        {
            continue;
        }
        let Ok(Some(row)) = summary(partition, id) else {
            continue;
        };
        rows.push(row);
        rows.sort_by_key(|row| std::cmp::Reverse((row.pinned, row.updated)));
        rows.truncate(MAX_RESTORED);
    }
    Ok(())
}

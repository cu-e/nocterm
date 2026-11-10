//! Per-scope discovery reads authoritative transcript headers directly.
use crate::{
    input::Input,
    namespace::{Partition, Root},
};
use nocterm_ai::{
    history::{self, ChatSummary, MAX_RESTORED},
    local_environment::ChatScope,
    storage::{AgentStorageError, require_local_scope},
};
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
        let summary = history::decode_summary(&bytes, id, stamp)?;
        if summary
            .as_ref()
            .is_some_and(|row| row.transcript_version().wire() != 1)
        {
            return Err(history::HistoryCodecError::InvalidHeader.into());
        }
        summary
    } else {
        let header = history::decode_catalog_header(&input.prefix()?, id)?;
        require_local_scope(partition.chat_scope(), &header.chat_scope())?;
        if header.transcript_version().wire() != partition.version() {
            return Err(history::HistoryCodecError::InvalidHeader.into());
        }
        header.summary(stamp)
    };
    if let Some(summary) = &summary {
        require_local_scope(partition.chat_scope(), &summary.chat_scope())?;
    }
    Ok(summary)
}
pub(super) fn load(root: &Root, scope: &ChatScope) -> Result<Vec<ChatSummary>, AgentStorageError> {
    let Some(directory) = root.open(false)? else {
        return Ok(Vec::new());
    };
    let partitions = crate::namespace::readable(&directory, scope)?;
    let mut rows = Vec::new();
    for (index, partition) in partitions.iter().enumerate() {
        scan(partition, &partitions[..index], &mut rows)?;
    }
    Ok(rows)
}
fn scan(
    partition: &Partition,
    preferred: &[Partition],
    rows: &mut Vec<ChatSummary>,
) -> Result<(), AgentStorageError> {
    let mut entries = partition.directory().entries()?;
    while let Some(name) = entries.next()? {
        let Some(id) = Partition::id(&name) else {
            continue;
        };
        let mut shadowed = false;
        for preferred in preferred {
            shadowed |= preferred.exists(id)?;
        }
        if shadowed {
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

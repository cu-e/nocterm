//! Rebuildable bounded sidecars. The transcript remains the source of truth.
pub(crate) mod publication;
use crate::{format, legacy, loading};
#[cfg(test)]
use nocterm_ai::history::SavedChat;
use nocterm_ai::history::{ChatSummary, FileStamp, MAX_FILE_BYTES, MAX_RESTORED};
use std::{
    fs,
    io::{BufReader, Read as _},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
const MAX_METADATA_BYTES: u64 = 16 * 1024;
fn cache_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.meta"))
}
fn stamp(metadata: &fs::Metadata) -> FileStamp {
    FileStamp {
        bytes: metadata.len(),
        modified_nanos: metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |time| time.as_nanos()),
    }
}
#[cfg(test)]
pub(super) fn update_cache(dir: &Path, chat: &SavedChat, queued: bool) {
    let Ok(path) = loading::file(dir, chat.id()) else {
        return;
    };
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    write_cache(
        dir,
        &ChatSummary::from_chat(chat).with_stamp(stamp(&metadata), queued),
    );
}
fn write_cache(dir: &Path, summary: &ChatSummary) {
    if let Ok(bytes) = serde_json::to_vec(summary) {
        let _ = nocterm_core::fs::write_private_atomic(&cache_path(dir, &summary.id), &bytes);
    }
}
pub(super) fn delete_cache(dir: &Path, id: &str) {
    let _ = fs::remove_file(cache_path(dir, id));
}
pub(super) fn load_catalog(dir: &Path) -> Vec<ChatSummary> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if path.extension().is_none_or(|ext| ext != "json") || loading::file(dir, id).is_err() {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            continue;
        }
        let file_stamp = stamp(&metadata);
        let cached = fs::File::open(cache_path(dir, id))
            .ok()
            .and_then(|file| {
                serde_json::from_reader::<_, ChatSummary>(BufReader::new(
                    file.take(MAX_METADATA_BYTES),
                ))
                .ok()
            })
            .filter(|row| row.matches_stamp(id, file_stamp));
        let summary = cached.or_else(|| {
            let bytes = loading::read(&path, id).ok()?;
            let summary = match format::classify(&bytes).ok()? {
                format::Format::Version2 => {
                    nocterm_ai::history::decode_summary(&bytes, id, file_stamp)
                }
                format::Format::LegacyV1 => legacy::decode_summary(&bytes, id, file_stamp),
            };
            summary.ok()?.inspect(|row| write_cache(dir, row))
        });
        if let Some(row) = summary.filter(|row| row.has_content || row.name.is_some() || row.pinned)
        {
            rows.push(row);
            rows.sort_by_key(|row| std::cmp::Reverse((row.pinned, row.updated)));
            rows.truncate(MAX_RESTORED);
        }
    }
    rows
}
#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

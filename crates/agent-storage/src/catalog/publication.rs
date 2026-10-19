//! Rebuildable metadata publication borrows the actual document framing.
use super::stamp;
use crate::writing::managed_writer::ManagedWire;
use nocterm_ai::{
    history::{
        ChatSummaryWire, EncodingDecision, HistoryEncoding, SavedDocument, physical::SnapshotOwner,
    },
    model::json::deferred::DecodeLedger,
};
use std::path::Path;

pub(crate) fn managed(dir: &Path, path: &Path, snapshot: &SnapshotOwner, ledger: &DecodeLedger) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    let summary = ChatSummaryWire::from_managed(snapshot, stamp(&metadata));
    write(dir, &snapshot.id(), &summary, ledger);
}
pub(crate) fn document(dir: &Path, path: &Path, document: &SavedDocument, ledger: &DecodeLedger) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    let summary = ChatSummaryWire::from_document(document, stamp(&metadata));
    write(dir, &document.id, &summary, ledger);
}
fn write(dir: &Path, id: &str, summary: &impl ManagedWire, ledger: &DecodeLedger) {
    let Ok(size) = nocterm_ai::history::measure_json(summary) else {
        return;
    };
    let Ok(path) = crate::writing::native_paths::document(dir, id, ".meta", ledger) else {
        return;
    };
    let decision = EncodingDecision {
        encoding: HistoryEncoding::Version2,
        size,
    };
    if let Ok(stage) =
        crate::writing::native_stage::stage(path.as_path(), ledger, summary, decision)
    {
        let _ = stage.publish();
    }
}

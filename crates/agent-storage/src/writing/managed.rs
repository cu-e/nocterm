//! Native publication borrows the original Active payload for its whole write.
use crate::{catalog, legacy, loading};
use nocterm_ai::{
    history::{
        self, HistoryEncoding,
        physical::{SnapshotOwner, SnapshotWire},
        writing::{ManagedWrite, ManagedWriteIntent},
    },
    storage::AgentStorageError,
};
use std::path::Path;

pub(crate) fn save(dir: &Path, write: ManagedWrite) -> Result<(), AgentStorageError> {
    write.write(|ledger, snapshot, intent| match intent {
        ManagedWriteIntent::Snapshot => snapshot_write(dir, ledger, snapshot),
        ManagedWriteIntent::ArchiveDraft { updated } => {
            archive_draft(dir, ledger, snapshot, updated)
        }
    })
}
fn snapshot_write(
    dir: &Path,
    ledger: &nocterm_ai::model::json::deferred::DecodeLedger,
    snapshot: &SnapshotOwner,
) -> Result<(), AgentStorageError> {
    let decision = history::decision_managed(snapshot)?;
    let path = super::native_paths::document(dir, &snapshot.id(), ".json", ledger)?;
    super::native_paths::private_directory(dir, ledger)?;
    let staged = match decision.encoding {
        HistoryEncoding::Version2 => super::native_stage::stage(
            path.as_path(),
            ledger,
            &SnapshotWire::new(snapshot),
            decision,
        ),
        HistoryEncoding::LegacyV1 => {
            let prepared = snapshot.prepared();
            let grammar = prepared
                .as_ref()
                .and_then(history::PreparedHistory::legacy)
                .ok_or(history::HistoryCodecError::CandidateMismatch)?;
            super::native_stage::stage(
                path.as_path(),
                ledger,
                &legacy::projection_managed(snapshot, decision, grammar.as_ref())?,
                decision,
            )
        }
    };
    staged?.publish()?;
    catalog::publication::managed(dir, path.as_path(), snapshot, ledger);
    Ok(())
}
fn archive_draft(
    dir: &Path,
    ledger: &nocterm_ai::model::json::deferred::DecodeLedger,
    snapshot: &SnapshotOwner,
    updated: u64,
) -> Result<(), AgentStorageError> {
    let original = loading::native::materialize(dir, &snapshot.id(), ledger)?;
    let (mut document, prepared) = original.prepare_document_in_writer(ledger)?;
    let draft = snapshot.composer().draft();
    if !nocterm_ai::history::native::archive::patch_archived_draft(
        &mut document,
        draft.as_deref(),
        updated,
    )? {
        return Ok(());
    }
    let decision = history::decision_document(&document, &prepared)?;
    let path = super::native_paths::document(dir, &snapshot.id(), ".json", ledger)?;
    let staged = match decision.encoding {
        HistoryEncoding::Version2 => super::native_stage::stage(
            path.as_path(),
            ledger,
            &history::NativeDocumentWire::new(&document)?,
            decision,
        ),
        HistoryEncoding::LegacyV1 => {
            let grammar = prepared
                .legacy()
                .ok_or(history::HistoryCodecError::CandidateMismatch)?;
            super::native_stage::stage(
                path.as_path(),
                ledger,
                &legacy::projection(document.get().view()?, decision, grammar.as_ref())?,
                decision,
            )
        }
    };
    staged?.publish()?;
    catalog::publication::document(dir, path.as_path(), document.get(), ledger);
    Ok(())
}

//! The original synchronous materializer runs inside its paid reader task Box.
use nocterm_ai::{
    history::{
        ManagedReaderFuture, NativeMaterialized, ReaderDocumentId, ReaderMaterializer,
        ReaderPreparationFailure, ReaderRequest,
    },
    model::json::deferred::DecodeLedger,
    storage::AgentStorageError,
};
use std::{path::PathBuf, sync::Arc};
struct FileReadTask {
    id: ReaderDocumentId,
    directory: Arc<PathBuf>,
}
impl ReaderMaterializer for FileReadTask {
    fn materialize(
        &mut self,
        ledger: &DecodeLedger,
    ) -> Result<NativeMaterialized, AgentStorageError> {
        super::native::materialize(&self.directory, self.id.as_str(), ledger)
    }
}
#[expect(
    clippy::result_large_err,
    reason = "refusals return ledger-paid values; a Box would be an unpaid allocation"
)]
pub(crate) fn prepare(
    directory: Arc<PathBuf>,
    id: ReaderDocumentId,
    request: ReaderRequest,
) -> Result<ManagedReaderFuture, ReaderPreparationFailure> {
    request.prepare_native(id, move |id, _| Ok(FileReadTask { id, directory }))
}

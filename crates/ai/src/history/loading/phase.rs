//! Owned native reader phase. Production callers cannot extract its scoped ledger.
use super::{NativeLoadError, ReaderCompletion};
use crate::{
    history::{
        HistoryOrigin, native::NativeChat, physical::SnapshotBuilder, retention::LegacyFormat,
    },
    model::{
        json::deferred::DecodeLedger,
        retention::{
            RetentionError,
            document::{ConstructionRole, RetainedRole},
        },
    },
    storage::AgentStorageError,
};

/// Field order applies to cancellation and unwinding too: delivery payload,
/// native ledger metadata, then the actual construction/account lifetime pin.
pub struct NativePhase {
    delivery: RetainedRole,
    ledger: DecodeLedger,
    reader: ConstructionRole,
}
impl NativePhase {
    #[expect(
        clippy::result_large_err,
        reason = "boxing the refusal would be an allocation the ledger never paid"
    )]
    pub fn begin(
        mut reader: ConstructionRole,
        delivery: RetainedRole,
    ) -> Result<Self, ReaderCompletion> {
        if !reader.is_reader()
            || !delivery.is_delivery()
            || delivery.snapshot().is_some()
            || !reader.scope().same_document(delivery.scope())
        {
            return Err(ReaderCompletion::failed(
                reader,
                delivery,
                RetentionError::ForeignAuthority.into(),
            ));
        }
        if let Err(error) = reader.begin_phase() {
            return Err(ReaderCompletion::failed(reader, delivery, error.into()));
        }
        match DecodeLedger::for_operation(&reader, crate::history::MAX_RESIDENT_BYTES) {
            Ok(ledger) => Ok(Self {
                delivery,
                ledger,
                reader,
            }),
            Err(error) => {
                reader.close_phase();
                Err(ReaderCompletion::failed(
                    reader,
                    delivery,
                    crate::history::HistoryCodecError::from(error).into(),
                ))
            }
        }
    }

    /// Runs the actual adapter materializer without holding a document or census
    /// lock. Its pending locals must be consumed or dropped before returning.
    /// The returned terminal owns the complete native chat through publication.
    pub fn read(
        mut self,
        materialize: impl FnOnce(&DecodeLedger) -> Result<NativeMaterialized, AgentStorageError>,
    ) -> ReaderCompletion {
        let result = self.run(materialize);
        self.finish(result)
    }
    pub(super) fn stage_native<T>(&self, stage: impl FnOnce(&DecodeLedger) -> T) -> T {
        stage(&self.ledger)
    }
    pub(super) fn run(
        &mut self,
        materialize: impl FnOnce(&DecodeLedger) -> Result<NativeMaterialized, AgentStorageError>,
    ) -> Result<(), NativeLoadError> {
        let NativeMaterialized {
            chat,
            origin,
            grammar,
            source_resident,
        } = materialize(&self.ledger)?;
        let (document, prepared) = chat.prepare_document(origin, grammar, source_resident)?;
        self.publish(document, prepared)
    }
    pub(super) fn finish(self, result: Result<(), NativeLoadError>) -> ReaderCompletion {
        match result {
            Ok(()) => {
                let Self {
                    delivery,
                    ledger,
                    mut reader,
                } = self;
                drop(ledger);
                reader.close_phase();
                ReaderCompletion::ready(reader, delivery)
            }
            Err(error) => self.failure(error),
        }
    }
    pub fn fail(self, error: AgentStorageError) -> ReaderCompletion {
        self.failure(error.into())
    }

    fn publish(
        &mut self,
        document: crate::model::native::NativeOwned<crate::history::SavedDocument>,
        prepared: crate::history::PreparedHistory,
    ) -> Result<(), NativeLoadError> {
        let staged =
            SnapshotBuilder::from_document(&self.reader, document)?.finish(Some(prepared))?;
        self.delivery.replace(staged)?;
        Ok(())
    }

    fn failure(self, error: NativeLoadError) -> ReaderCompletion {
        let Self {
            delivery,
            ledger,
            mut reader,
        } = self;
        drop(ledger);
        reader.close_phase();
        ReaderCompletion::failed(reader, delivery, error)
    }
}
/// Terminal native result; pending payload fields precede their metadata owners.
/// This value is consumed immediately by `NativePhase::read`.
pub struct NativeMaterialized {
    chat: NativeChat,
    grammar: Option<LegacyFormat>,
    origin: HistoryOrigin,
    source_resident: usize,
}
impl NativeMaterialized {
    /// Materializes an archive patch under the original writer's native phase.
    /// It creates neither reader capabilities nor a replacement document bank.
    pub fn prepare_document_in_writer(
        self,
        ledger: &DecodeLedger,
    ) -> Result<
        (
            crate::model::native::NativeOwned<crate::history::SavedDocument>,
            crate::history::PreparedHistory,
        ),
        crate::history::HistoryCodecError,
    > {
        if !self.chat.belongs_to(ledger) {
            return Err(crate::model::json::deferred::JsonDecodeError::ForeignLedger.into());
        }
        self.chat
            .prepare_document(self.origin, self.grammar, self.source_resident)
    }
    pub fn new(
        chat: NativeChat,
        origin: HistoryOrigin,
        grammar: Option<LegacyFormat>,
        source_resident: usize,
    ) -> Self {
        Self {
            chat,
            grammar,
            origin,
            source_resident,
        }
    }
}

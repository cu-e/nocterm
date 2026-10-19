//! Initial framing prepays its real document copy from supplied identity facts.
use crate::{
    history::{self, SavedDocument},
    model::{
        json::deferred::{DecodeLedger, JsonDecodeError},
        native::{NativeList, NativeOwned},
        retention::document::ConstructionRole,
    },
};
mod attachment_edit;
pub(super) use attachment_edit::AttachmentPreparation;
pub use attachment_edit::{AttachmentEdit, LiveAttachmentRef, LocalAttachmentFact};

pub(super) struct DocumentSeed<'a> {
    agent: &'a str,
    id: &'a str,
    updated: u64,
}
impl<'a> DocumentSeed<'a> {
    pub(super) fn new(agent: &'a str, id: &'a str, updated: u64) -> Self {
        Self { agent, id, updated }
    }
    pub(super) fn prepare(
        self,
        role: &ConstructionRole,
    ) -> Result<NativeOwned<SavedDocument>, history::HistoryCodecError> {
        let ledger = DecodeLedger::for_operation(role, history::MAX_RESIDENT_BYTES)?;
        let id = copied(&ledger, self.id)?;
        let metadata = crate::history::native::document::metadata(
            copied(&ledger, self.agent)?,
            NativeOwned::absent(&ledger)?,
            NativeOwned::absent(&ledger)?,
            false,
            self.updated,
            NativeOwned::absent(&ledger)?,
        )?;
        let resume = crate::history::native::document::resume(
            NativeOwned::absent(&ledger)?,
            NativeOwned::absent(&ledger)?,
            crate::history::native::document::empty_choices(&ledger)?,
            NativeOwned::absent(&ledger)?,
            false,
        )?;
        let transcript = crate::history::native::document::transcript(
            NativeList::new(&ledger, 0)?.finish(),
            NativeList::new(&ledger, 0)?.finish(),
        )?;
        let composer = crate::history::native::document::composer(
            NativeOwned::absent(&ledger)?,
            NativeList::new(&ledger, 0)?.finish(),
            NativeList::new(&ledger, 0)?.finish(),
        )?;
        Ok(
            history::native::NativeChat::new(id, metadata, resume, transcript, composer)?
                .into_document(),
        )
    }
}
fn copied(ledger: &DecodeLedger, text: &str) -> Result<NativeOwned<String>, JsonDecodeError> {
    let allocation = ledger.reserve(text.len())?;
    let mut value = String::new();
    value.try_reserve_exact(text.len())?;
    if value.capacity() != text.len() {
        return Err(JsonDecodeError::CapacityProfile);
    }
    value.push_str(text);
    Ok(NativeOwned::from_parts(value, allocation))
}

pub(super) struct ReaderIdentity<'a>(&'a str);
impl<'a> ReaderIdentity<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        Self(text)
    }
    pub(super) fn prepare(
        self,
        role: &ConstructionRole,
    ) -> Result<crate::history::ReaderDocumentId, JsonDecodeError> {
        let ledger = DecodeLedger::for_operation(role, history::MAX_RESIDENT_BYTES)?;
        copied(&ledger, self.0).map(crate::history::ReaderDocumentId::from_native)
    }
}

pub(crate) struct QueuedText<'a> {
    text: &'a str,
    attachments: &'a [super::inputs::SavedAttachmentRef<'a>],
}
impl<'a> QueuedText<'a> {
    pub(crate) fn new(
        text: &'a str,
        attachments: &'a [super::inputs::SavedAttachmentRef<'a>],
    ) -> Self {
        Self { text, attachments }
    }
    pub(crate) fn prepare(
        self,
        role: &ConstructionRole,
    ) -> Result<
        (
            NativeOwned<String>,
            NativeOwned<Vec<history::SavedAttachment>>,
        ),
        JsonDecodeError,
    > {
        let ledger = DecodeLedger::for_operation(role, history::MAX_RESIDENT_BYTES)?;
        let text = copied(&ledger, self.text)?;
        let mut attachments = NativeList::new(&ledger, self.attachments.len())?;
        for attachment in self.attachments {
            use super::inputs::SavedAttachmentRef;
            let (text, allocation) = copied(
                &ledger,
                match attachment {
                    SavedAttachmentRef::Connection(text)
                    | SavedAttachmentRef::Group(text)
                    | SavedAttachmentRef::LocalTerminal(text) => text,
                },
            )?
            .into_parts();
            let value = match attachment {
                SavedAttachmentRef::Connection(_) => history::SavedAttachment::Connection(text),
                SavedAttachmentRef::Group(_) => history::SavedAttachment::Group(text),
                SavedAttachmentRef::LocalTerminal(_) => {
                    history::SavedAttachment::LocalTerminal(text)
                }
            };
            attachments.push(NativeOwned::from_parts(value, allocation))?;
        }
        Ok((text, attachments.finish()))
    }
}

mod local_text;
pub(super) use local_text::{LocalTextEdit, TextTarget};

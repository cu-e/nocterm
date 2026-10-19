//! Concrete history records consume guarded native fields without cloning them.
use crate::{
    history::*,
    model::{
        json::deferred::{DecodeLedger, JsonDecodeError},
        native::NativeOwned,
    },
    session_config::Choices,
    thread::Entry,
};
use std::path::PathBuf;
pub fn metadata(
    agent_id: NativeOwned<String>,
    title: NativeOwned<Option<String>>,
    name: NativeOwned<Option<String>>,
    pinned: bool,
    updated: u64,
    model: NativeOwned<Option<String>>,
) -> Result<NativeOwned<ChatMetadata>, JsonDecodeError> {
    let ((((agent_id, title), name), model), allocation) =
        agent_id.join(title)?.join(name)?.join(model)?.into_parts();
    Ok(NativeOwned::from_parts(
        ChatMetadata {
            agent_id,
            title,
            name,
            pinned,
            updated,
            model,
        },
        allocation,
    ))
}
pub fn resume(
    session_id: NativeOwned<Option<String>>,
    workdir: NativeOwned<Option<PathBuf>>,
    config: NativeOwned<Choices>,
    mode: NativeOwned<Option<String>>,
    pending_history: bool,
) -> Result<NativeOwned<ResumeDescriptor>, JsonDecodeError> {
    let ((((session_id, workdir), config), mode), allocation) = session_id
        .join(workdir)?
        .join(config)?
        .join(mode)?
        .into_parts();
    Ok(NativeOwned::from_parts(
        ResumeDescriptor {
            session_id,
            workdir,
            config,
            mode,
            pending_history,
        },
        allocation,
    ))
}
pub fn transcript(
    entries: NativeOwned<Vec<Entry>>,
    times: NativeOwned<Vec<u64>>,
) -> Result<NativeOwned<TranscriptData>, JsonDecodeError> {
    let ((entries, times), allocation) = entries.join(times)?.into_parts();
    Ok(NativeOwned::from_parts(
        TranscriptData { entries, times },
        allocation,
    ))
}
pub fn composer(
    draft: NativeOwned<Option<String>>,
    queue: NativeOwned<Vec<SavedPrompt>>,
    attachments: NativeOwned<Vec<SavedAttachment>>,
) -> Result<NativeOwned<ComposerData>, JsonDecodeError> {
    let (((draft, queue), attachments), allocation) =
        draft.join(queue)?.join(attachments)?.into_parts();
    Ok(NativeOwned::from_parts(
        ComposerData {
            draft,
            queue,
            attachments,
        },
        allocation,
    ))
}
pub fn prompt(
    id: u64,
    text: NativeOwned<String>,
    images: NativeOwned<Vec<crate::model::content::Content>>,
    attachments: NativeOwned<Vec<SavedAttachment>>,
) -> Result<NativeOwned<SavedPrompt>, JsonDecodeError> {
    let (((text, images), attachments), allocation) =
        text.join(images)?.join(attachments)?.into_parts();
    Ok(NativeOwned::from_parts(
        SavedPrompt {
            id,
            text,
            images,
            attachments,
        },
        allocation,
    ))
}
pub fn connection(id: NativeOwned<String>) -> NativeOwned<SavedAttachment> {
    let (id, allocation) = id.into_parts();
    NativeOwned::from_parts(SavedAttachment::Connection(id), allocation)
}
pub fn group(id: NativeOwned<String>) -> NativeOwned<SavedAttachment> {
    let (id, allocation) = id.into_parts();
    NativeOwned::from_parts(SavedAttachment::Group(id), allocation)
}
pub fn local_terminal(id: NativeOwned<String>) -> NativeOwned<SavedAttachment> {
    let (id, allocation) = id.into_parts();
    NativeOwned::from_parts(SavedAttachment::LocalTerminal(id), allocation)
}
pub fn empty_choices(ledger: &DecodeLedger) -> Result<NativeOwned<Choices>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(Choices::new(), ledger.reserve(0)?))
}

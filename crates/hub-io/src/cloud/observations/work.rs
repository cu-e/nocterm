//! Closed SDK metadata and crypto generations, independent of native result data.
use nocterm_hub_model::cloud::{
    MAX_HEADER_BYTES, MAX_ID_BYTES, MAX_PLAINTEXT_BYTES,
    events::{
        AcceptedPrompt, BackgroundJob, BackgroundLifecyclePayload, CloudEvent,
        MAX_EVENT_JSON_BYTES, ToolIntentPayload, ToolObservationPayload,
    },
    payload::{ApprovalPayload, PromptPart},
};
use std::mem::size_of;

pub(super) fn scratch(ciphertext: usize, count: usize) -> Option<usize> {
    if ciphertext > MAX_PLAINTEXT_BYTES.checked_add(42)? || ciphertext < 42 {
        return None;
    }
    // Same canonical SDK decoder limits: 256 argv collected with geometric
    // growth, 128 exact prompt slots, <=6 owned 128-byte identifiers and one box.
    let boxed = [
        size_of::<AcceptedPrompt<'_>>(),
        size_of::<ToolIntentPayload<'_>>(),
        size_of::<ApprovalPayload>(),
        size_of::<ToolObservationPayload<'_>>() + size_of::<BackgroundJob<'_>>(),
        size_of::<BackgroundLifecyclePayload<'_>>(),
    ]
    .into_iter()
    .max()?;
    let per_event = boxed
        .checked_add(512usize.checked_mul(size_of::<&str>())?)?
        .checked_add(128usize.checked_mul(size_of::<PromptPart<'_>>())?)?
        .checked_add(6usize.checked_mul(MAX_ID_BYTES)?)?;
    // Signed-record plaintext, context/header/AAD canonical generations, one
    // bounded JSON validation parser and the SDK snapshot collect Vec coexist.
    ciphertext
        .checked_add(16usize.checked_mul(MAX_HEADER_BYTES)?)?
        .checked_add(count.checked_mul(per_event)?)?
        .checked_add(
            count
                .checked_mul(2)?
                .checked_mul(size_of::<CloudEvent<'_>>())?,
        )?
        .checked_add(ciphertext.min(MAX_EVENT_JSON_BYTES).checked_mul(2)?)
}

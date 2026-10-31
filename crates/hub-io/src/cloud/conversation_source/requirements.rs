//! Logical live-buffer components are checked before native/SDK materialization.
use super::heads;
use nocterm_cloud_model::projection::{ProjectionError, ports::ConversationCommand};
use nocterm_hub_client::cloud::{CloudReplay, CloudUpload};
use nocterm_hub_model::cloud::{MAX_ENVELOPE_BYTES, MAX_HEADER_BYTES};

const METADATA: usize = 16 * MAX_HEADER_BYTES;
fn for_action_body(
    action: &nocterm_ai::conversation::ConversationAction,
) -> Result<usize, ProjectionError> {
    let text = match action {
        nocterm_ai::conversation::ConversationAction::Prompt(text) => text.len(),
        nocterm_ai::conversation::ConversationAction::Stop => 0,
    };
    body_size(text)
}
fn body_size(text: usize) -> Result<usize, ProjectionError> {
    text.checked_mul(6)
        .and_then(|bytes| bytes.checked_add(METADATA))
        .filter(|bytes| *bytes <= MAX_ENVELOPE_BYTES)
        .ok_or(ProjectionError::Capacity)
}
pub(super) fn stage(command: &ConversationCommand) -> Result<usize, ProjectionError> {
    match &command.action {
        nocterm_cloud_model::projection::ConversationIntent::Prompt(text) => body_work(text.len()),
        nocterm_cloud_model::projection::ConversationIntent::Stop
        | nocterm_cloud_model::projection::ConversationIntent::AnswerApproval(_) => body_work(0),
        nocterm_cloud_model::projection::ConversationIntent::GrantProvider(access) => {
            body_work(access.resident_bytes())
        }
    }
}
pub(super) fn action(
    action: &nocterm_ai::conversation::ConversationAction,
) -> Result<usize, ProjectionError> {
    let body = for_action_body(action)?;
    work_for_body(body)
}
fn body_work(text: usize) -> Result<usize, ProjectionError> {
    work_for_body(body_size(text)?)
}
fn work_for_body(body: usize) -> Result<usize, ProjectionError> {
    let upload = CloudUpload::buffer_work_bytes(body).map_err(|_| ProjectionError::Capacity)?;
    // At seal time the public body/plaintext/cipher generations coexist. Native
    // storage additionally holds exact M, authenticated outbox plaintext/cipher,
    // and the private staged-file generation; four whole body slots bound either
    // peak. Fresh query and actual upload guards may retire independently.
    body.checked_mul(4)
        .and_then(|bytes| bytes.checked_add(upload))
        .and_then(|bytes| bytes.checked_add(heads::work_bytes()))
        .and_then(|bytes| bytes.checked_add(METADATA))
        .ok_or(ProjectionError::Capacity)
}
pub(super) fn read_capacity(work: usize) -> Result<usize, ProjectionError> {
    work.checked_sub(heads::work_bytes())
        .and_then(|bytes| bytes.checked_sub(METADATA))
        .map(|bytes| bytes / 4)
        .filter(|bytes| *bytes > 0)
        .ok_or(ProjectionError::Capacity)
}
pub(super) fn replay(maximum: usize) -> Result<usize, ProjectionError> {
    let sdk = CloudReplay::buffer_work_bytes();
    let decode = super::super::AccountObservationDecoder::work_bytes(maximum, false)
        .map_err(|_| ProjectionError::Capacity)?;
    // Received ciphertext, a second spooled/encrypted cache generation, one
    // current decoded value+destination clone and the existing retained page.
    maximum
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(decode))
        .and_then(|bytes| bytes.checked_add(2 * nocterm_cloud_model::projection::MAX_REPLAY_BYTES))
        .and_then(|bytes| bytes.checked_add(sdk))
        .and_then(|bytes| bytes.checked_add(heads::work_bytes()))
        .ok_or(ProjectionError::Capacity)
}

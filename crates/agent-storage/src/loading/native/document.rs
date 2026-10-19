//! Document scopes keep all final DTO capacity credits in the selected ledger.
use super::{
    Error, content, fields as f, lists,
    schema::{self as s, Scope},
    tools,
};
use crate::loading::{
    held_file::{HeldFile, plan::AuthenticatedPlan},
    json_fields::{Key, Key::*},
};
use nocterm_ai::{
    history::{
        ChatMetadata, ComposerData, ResumeDescriptor, SavedAttachment, SavedPrompt, TranscriptData,
        native::NativeChat,
    },
    model::{
        json::deferred::{DecodeLedger, native::NativeKind},
        native::{NativeChoices, NativeOwned},
    },
    thread::Entry,
};
pub(super) fn document(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
    expected_id: &str,
) -> std::result::Result<NativeChat, Error> {
    let id = f::text(plan, held, Id, ledger)?;
    if id.as_str() != expected_id {
        return Err(nocterm_ai::history::HistoryCodecError::IdentityMismatch.into());
    }
    let metadata_plan = f::child(plan, held, Metadata, s::METADATA, ledger)?;
    let metadata = metadata(&metadata_plan, held, ledger)?;
    let resume_plan = f::child(plan, held, Resume, s::RESUME, ledger)?;
    let resume = resume(&resume_plan, held, ledger)?;
    let transcript_plan = f::child(plan, held, Transcript, s::TRANSCRIPT, ledger)?;
    let transcript = transcript(&transcript_plan, held, ledger)?;
    let composer_plan = f::child(plan, held, Composer, s::COMPOSER, ledger)?;
    let composer = composer(&composer_plan, held, ledger)?;
    Ok(NativeChat::new(id, metadata, resume, transcript, composer)?)
}
fn metadata(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<ChatMetadata>, Error> {
    let agent_id = f::text(plan, held, AgentId, ledger)?;
    let title = f::optional_text(plan, held, Title, ledger)?;
    let name = f::optional_text(plan, held, Name, ledger)?;
    let pinned = f::boolean(plan, held, Pinned, ledger)?;
    let updated = f::unsigned(plan, held, Updated, ledger)?;
    let model = f::optional_text(plan, held, Model, ledger)?;
    Ok(nocterm_ai::history::native::document::metadata(
        agent_id, title, name, pinned, updated, model,
    )?)
}
fn resume(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<ResumeDescriptor>, Error> {
    let session = f::optional_text(plan, held, SessionId, ledger)?;
    let workdir = f::optional(plan, held, Workdir, ledger, f::path)?;
    let config = choices(plan, held, ledger)?;
    let mode = f::optional_text(plan, held, Mode, ledger)?;
    let pending = f::boolean(plan, held, PendingHistory, ledger)?;
    Ok(nocterm_ai::history::native::document::resume(
        session, workdir, config, mode, pending,
    )?)
}
pub(super) fn choices(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<nocterm_ai::session_config::Choices>, Error> {
    lists::validate_members(plan, held, Config, ledger, None, |p, h, member, l| {
        let node = crate::loading::literal_reader::decode_authenticated_native(
            p,
            h,
            |_| member.native_span(),
            l,
        )??;
        if matches!(node.kind(), NativeKind::Boolean | NativeKind::String) {
            Ok(())
        } else {
            Err(Error::Type)
        }
    })?;
    let node = f::node(plan, held, Config, ledger)?;
    if node.kind() != NativeKind::Object {
        return Err(Error::Type);
    }
    let count = node.container_len().ok_or(Error::Type)?;
    let mut children = node.into_children()?;
    let mut choices = NativeChoices::new(ledger, count)?;
    while let Some((key, value)) = children.next_child()? {
        let key = NativeOwned::text(key.ok_or(Error::Type)?);
        let value = match value.kind() {
            NativeKind::Boolean => nocterm_ai::model::native::choices::flag(
                value.boolean().ok_or(Error::Type)?,
                ledger,
            )?,
            NativeKind::String => nocterm_ai::model::native::choices::text_value(
                NativeOwned::text(value.into_text()?),
            ),
            _ => return Err(Error::Type),
        };
        choices.push(key, value)?;
    }
    Ok(choices.finish()?)
}
fn transcript(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<TranscriptData>, Error> {
    let entries = lists::list(plan, held, Entries, ledger, 2000, s::TAG, entry)?;
    let times = lists::primitive(plan, held, Times, ledger, 2000, |node, l| {
        let value = node.number().and_then(|n| n.as_u64()).ok_or(Error::Type)?;
        Ok(NativeOwned::scalar(value, l)?)
    })?;
    Ok(nocterm_ai::history::native::document::transcript(
        entries, times,
    )?)
}
fn entry(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<Entry>, Error> {
    let kind = f::tag(plan, held, ledger)?;
    match kind.as_str() {
        "agent" => Ok(nocterm_ai::history::native::entries::agent(f::text(
            plan, held, Value, ledger,
        )?)),
        "thought" => Ok(nocterm_ai::history::native::entries::thought(f::text(
            plan, held, Value, ledger,
        )?)),
        "user" => Ok(nocterm_ai::history::native::entries::user(lists::list(
            plan,
            held,
            Value,
            ledger,
            usize::MAX,
            s::TAG,
            content::content,
        )?)),
        "content" => {
            let p = f::child(plan, held, Value, s::TAG, ledger)?;
            Ok(nocterm_ai::history::native::entries::content_entry(
                content::content(&p, held, ledger)?,
            ))
        }
        "tool" => {
            let p = f::child(plan, held, Value, s::TOOL, ledger)?;
            Ok(nocterm_ai::history::native::entries::tool_entry(
                tools::tool(&p, held, ledger)?,
            ))
        }
        _ => Err(Error::Variant),
    }
}
fn composer(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<ComposerData>, Error> {
    let draft = f::optional_text(plan, held, Draft, ledger)?;
    let queue = lists::list(plan, held, Queue, ledger, 1024, s::PROMPT, prompt)?;
    let attachments = attachments(plan, held, Attachments, ledger)?;
    Ok(nocterm_ai::history::native::document::composer(
        draft,
        queue,
        attachments,
    )?)
}
fn prompt(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<SavedPrompt>, Error> {
    let id = f::unsigned(plan, held, Id, ledger)?;
    let text = f::text(plan, held, Text, ledger)?;
    let images = lists::list(plan, held, Images, ledger, 128, s::TAG, content::content)?;
    Ok(nocterm_ai::history::native::document::prompt(
        id,
        text,
        images,
        attachments(plan, held, Attachments, ledger)?,
    )?)
}
pub(super) fn attachments(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<Vec<SavedAttachment>>, Error> {
    lists::list(plan, held, key, ledger, 1024, s::ATTACHMENT, |p, h, l| {
        let kind = f::tag(p, h, l)?;
        let id = f::text(p, h, Id, l)?;
        Ok(match kind.as_str() {
            "connection" => nocterm_ai::history::native::document::connection(id),
            "group" => nocterm_ai::history::native::document::group(id),
            "local_terminal" => nocterm_ai::history::native::document::local_terminal(id),
            _ => return Err(Error::Variant),
        })
    })
}

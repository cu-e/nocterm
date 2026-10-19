//! Flat v1 documents reuse neutral guarded DTO constructors without SDK DTOs.
use super::{Error, content, f, tools};
use crate::loading::{
    held_file::{HeldFile, plan::AuthenticatedPlan},
    json_fields::Key::*,
    native::{
        lists,
        schema::{self as s, Scope},
    },
};
use nocterm_ai::{
    history::{SavedPrompt, native::NativeChat},
    model::{json::deferred::DecodeLedger, native::NativeOwned},
    thread::Entry,
};
pub(in crate::loading::native) fn document(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
    expected_id: &str,
    grammar: &mut crate::legacy::ControlledGrammar,
) -> std::result::Result<NativeChat, Error> {
    let id = f::text(plan, held, Id, ledger)?;
    if id.as_str() != expected_id {
        return Err(nocterm_ai::history::HistoryCodecError::IdentityMismatch.into());
    }
    let agent = f::text(plan, held, AgentId, ledger)?;
    let title = f::optional_text(plan, held, Title, ledger)?;
    let name = f::optional_text(plan, held, Name, ledger)?;
    let pinned = if plan.payload().has(Pinned) {
        f::boolean(plan, held, Pinned, ledger)?
    } else {
        false
    };
    let updated = f::unsigned(plan, held, Updated, ledger)?;
    let model = f::optional_text(plan, held, Model, ledger)?;
    let metadata = nocterm_ai::history::native::document::metadata(
        agent, title, name, pinned, updated, model,
    )?;
    let session = f::optional_text(plan, held, SessionId, ledger)?;
    let workdir = f::optional(plan, held, Workdir, ledger, f::path)?;
    let config = if plan.payload().has(Config) {
        super::super::document::choices(plan, held, ledger)?
    } else {
        nocterm_ai::history::native::document::empty_choices(ledger)?
    };
    let mode = f::optional_text(plan, held, Mode, ledger)?;
    let pending = if plan.payload().has(PendingHistory) {
        f::boolean(plan, held, PendingHistory, ledger)?
    } else {
        false
    };
    let resume =
        nocterm_ai::history::native::document::resume(session, workdir, config, mode, pending)?;
    let entries = lists::list(plan, held, Entries, ledger, 2000, s::TAG, |p, h, l| {
        entry(p, h, l, grammar)
    })?;
    let times = lists::primitive(plan, held, Times, ledger, 2000, |node, l| {
        let time = node.number().and_then(|v| v.as_u64()).ok_or(Error::Type)?;
        Ok(NativeOwned::scalar(time, l)?)
    })?;
    let transcript = nocterm_ai::history::native::document::transcript(entries, times)?;
    let draft = f::optional_text(plan, held, Draft, ledger)?;
    let queue = lists::list(plan, held, Queue, ledger, 1024, s::PROMPT, prompt)?;
    let attachments = super::super::document::attachments(plan, held, Attachments, ledger)?;
    let composer = nocterm_ai::history::native::document::composer(draft, queue, attachments)?;
    Ok(NativeChat::new(id, metadata, resume, transcript, composer)?)
}
fn entry(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
    grammar: &mut crate::legacy::ControlledGrammar,
) -> std::result::Result<NativeOwned<Entry>, Error> {
    let kind = f::text(plan, held, Kind, ledger)?;
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
            s::LEGACY_BLOCK,
            content::content,
        )?)),
        "content" => {
            let p = f::child(plan, held, Value, s::LEGACY_BLOCK, ledger)?;
            Ok(nocterm_ai::history::native::entries::content_entry(
                content::content(&p, held, ledger)?,
            ))
        }
        "tool" => {
            let p = f::child(plan, held, Value, s::LEGACY_CALL, ledger)?;
            let (tool, order) = tools::tool(&p, held, ledger)?;
            grammar.record(order)?;
            Ok(nocterm_ai::history::native::entries::tool_entry(tool))
        }
        _ => Err(Error::Variant),
    }
}
fn prompt(
    plan: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeOwned<SavedPrompt>, Error> {
    let id = f::unsigned(plan, held, Id, ledger)?;
    let text = f::text(plan, held, Text, ledger)?;
    let images = lists::list(
        plan,
        held,
        Images,
        ledger,
        128,
        s::LEGACY_BLOCK,
        content::content,
    )?;
    let attachments = super::super::document::attachments(plan, held, Attachments, ledger)?;
    Ok(nocterm_ai::history::native::document::prompt(
        id,
        text,
        images,
        attachments,
    )?)
}

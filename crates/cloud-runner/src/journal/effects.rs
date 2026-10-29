mod background;
pub(super) use background::cleanup as cleanup_background;
mod state;

use super::{Driver, codec, head, integer, storage, unsigned};
use nocterm_cloud_model::{
    authority::{StreamBinding, TargetBinding},
    identity::*,
    journal::*,
};
use rusqlite::{Connection, OptionalExtension, params};
use state::Effect;
use std::collections::BTreeSet;

pub(super) fn apply(
    db: &Connection,
    cipher: &dyn JournalCipher,
    batch: &JournalBatch,
    record: &JournalRecord,
    sequence: JournalSequence,
) -> Result<(), JournalError> {
    let stream = batch.lease().stream();
    let id = match record.kind() {
        JournalRecordKind::Intent(intent) => intent.id(),
        JournalRecordKind::Dispatched { intent }
        | JournalRecordKind::Declined { intent }
        | JournalRecordKind::Outcome { intent, .. }
        | JournalRecordKind::Reconciled { intent, .. }
        | JournalRecordKind::BackgroundCommand { intent, .. } => *intent,
        _ => return Ok(()),
    };
    let mut effect = match record.kind() {
        JournalRecordKind::Intent(intent) => {
            if load(db, cipher, stream, id)?.is_some() {
                return Err(JournalError::Conflict);
            }
            Effect::prepared(intent)
        }
        _ => load(db, cipher, stream, id)?.ok_or(JournalError::Conflict)?,
    };
    let stamp = || record.context().stamp().ok_or(JournalError::InvalidBatch);
    match record.kind() {
        JournalRecordKind::Dispatched { .. } => effect.dispatch(stamp()?)?,
        JournalRecordKind::Declined { .. } => effect.declined(stamp()?)?,
        JournalRecordKind::Outcome { outcome, .. } => effect.outcome(stamp()?, *outcome)?,
        JournalRecordKind::Reconciled { command, .. } => {
            if !batch.command().is_some_and(|accepted| {
                accepted.id() == *command && accepted.kind() == CommandKind::ResolveApproval
            }) {
                return Err(JournalError::InvalidBatch);
            }
            let original = effect.intent()?;
            let run = record.context().run().ok_or(JournalError::InvalidBatch)?;
            if !original.authority().run().same_run(run) {
                return Err(JournalError::Conflict);
            }
            if record.context().stamp().is_some() {
                effect.reconciled()?;
            } else {
                effect.declared_reconciled(batch.lease().epoch())?;
            }
        }
        JournalRecordKind::BackgroundCommand { .. } => return Ok(()), // Changed by the native lifecycle transaction before append.
        _ => {}
    }
    effect.validate()?;
    save(
        db,
        cipher,
        stream,
        id,
        sequence,
        batch.next_position()?.revision(),
        batch.lease().epoch(),
        &effect,
    )
}

fn load(
    db: &Connection,
    cipher: &dyn JournalCipher,
    stream: &StreamBinding,
    id: IntentId,
) -> Result<Option<Effect>, JournalError> {
    let row = db.query_row("SELECT state,sequence,revision,writer,length(body) FROM effects WHERE chat=?1 AND intent=?2", params![head::key(stream).as_slice(),id.as_bytes().as_slice()], |row|Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?,row.get::<_,i64>(3)?,row.get::<_,i64>(4)?))).optional().map_err(storage)?;
    let Some((status, sequence, revision, writer, length)) = row else {
        return Ok(None);
    };
    if head::check_length(length)? > 16 * 1024 {
        return Err(JournalError::Corrupt);
    }
    let context = JournalCipherContext::new(
        stream.clone(),
        JournalStorageClass::Effect,
        *id.as_bytes(),
        JournalSequence::from_u64(unsigned(sequence)?),
        StreamRevision::from_u64(unsigned(revision)?),
        WriterEpoch::new(unsigned(writer)?)?,
    );
    let body: Vec<u8> = db
        .query_row(
            "SELECT body FROM effects WHERE chat=?1 AND intent=?2",
            params![head::key(stream).as_slice(), id.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .map_err(storage)?;
    let effect = codec::open::<Effect>(cipher, &context, &body)?;
    effect.validate()?;
    let intent = effect.intent()?;
    if intent.id() != id
        || intent.authority().run().stream() != *stream
        || status != i64::from(effect.status())
    {
        return Err(JournalError::Corrupt);
    }
    Ok(Some(effect))
}
fn save(
    db: &Connection,
    cipher: &dyn JournalCipher,
    stream: &StreamBinding,
    id: IntentId,
    sequence: JournalSequence,
    revision: StreamRevision,
    writer: WriterEpoch,
    effect: &Effect,
) -> Result<(), JournalError> {
    let context = JournalCipherContext::new(
        stream.clone(),
        JournalStorageClass::Effect,
        *id.as_bytes(),
        sequence,
        revision,
        writer,
    );
    let body = codec::seal(cipher, &context, effect)?;
    if body.len() > 16 * 1024 {
        return Err(JournalError::Capacity);
    }
    db.execute("INSERT INTO effects(chat,scope,chat_id,runner,key_epoch,intent,state,sequence,revision,writer,body) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(chat,intent) DO UPDATE SET state=excluded.state,sequence=excluded.sequence,revision=excluded.revision,writer=excluded.writer,body=excluded.body",params![head::key(stream).as_slice(),head::scope_key(stream.scope()).as_slice(),stream.chat().as_bytes().as_slice(),stream.runner().as_bytes().as_slice(),integer(stream.runner_epoch().get())?,id.as_bytes().as_slice(),effect.status(),integer(sequence.get())?,integer(revision.get())?,integer(writer.get())?,body]).map_err(storage)?;
    Ok(())
}

pub(super) fn active_after(
    db: &Connection,
    cipher: &dyn JournalCipher,
    current: &head::Head,
    batch: &JournalBatch,
) -> Result<Vec<[u8; 16]>, JournalError> {
    let mut candidates = current.active().iter().copied().collect::<BTreeSet<_>>();
    for record in batch.records() {
        match record.kind() {
            JournalRecordKind::Intent(intent) => {
                candidates.insert(*intent.id().as_bytes());
            }
            JournalRecordKind::Dispatched { intent }
            | JournalRecordKind::Declined { intent }
            | JournalRecordKind::Outcome { intent, .. }
            | JournalRecordKind::Reconciled { intent, .. }
            | JournalRecordKind::BackgroundCommand { intent, .. } => {
                candidates.insert(*intent.as_bytes());
            }
            _ => {}
        }
    }
    let mut active = Vec::new();
    for id in candidates {
        let effect = load(db, cipher, batch.lease().stream(), IntentId::from_bytes(id))?
            .ok_or(JournalError::Corrupt)?;
        if effect.unresolved()?.is_some() {
            active.push(id);
        }
    }
    if active.len() > 256 {
        return Err(JournalError::Capacity);
    }
    Ok(active)
}

pub(super) fn unresolved(
    driver: &mut Driver,
    scope: &AccountScope,
    target: &TargetBinding,
    limit: usize,
) -> Result<Vec<UnresolvedTargetEffect>, JournalError> {
    if limit == 0 || limit > 256 {
        return Err(JournalError::Capacity);
    }
    driver.database.validate_leaves()?;
    let db = driver.connection.transaction().map_err(storage)?;
    if driver.catalog.binding().scope() != scope {
        return Err(JournalError::Fenced);
    }
    let catalog = driver.catalog.load(&db, driver.catalog_cipher.as_ref())?;
    driver.catalog.verify_rows(&db, &catalog)?;
    let mut matches = Vec::new();
    let mut inspected = 0usize;
    for stream in catalog.streams(driver.catalog.binding()) {
        let head = head::required(&db, driver.cipher.as_ref(), &stream)?;
        for id in head.active() {
            inspected += 1;
            if inspected > 1024 {
                return Err(JournalError::Capacity);
            }
            let effect = load(
                &db,
                driver.cipher.as_ref(),
                &stream,
                IntentId::from_bytes(*id),
            )?
            .ok_or(JournalError::Corrupt)?;
            let mut pending = effect.unresolved()?.ok_or(JournalError::Corrupt)?;
            if effect.writer_was_replaced(head.lease().epoch())? {
                pending = UnresolvedTargetEffect::new(
                    pending.intent().clone(),
                    pending
                        .background()
                        .map(|(terminal, command)| (terminal.to_owned(), command.to_owned())),
                    true,
                )?;
            }
            let original = pending.intent().authority().session().target();
            if original.host() == target.host()
                && original.principal() == target.principal()
                && original.workspace() == target.workspace()
            {
                if matches.len() == limit {
                    return Err(JournalError::Capacity);
                }
                matches.push(pending);
            }
        }
    }
    driver.catalog.observe(&catalog)?;
    Ok(matches)
}

pub(super) fn retain(
    driver: &mut Driver,
    command: &nocterm_cloud_model::tool::BackgroundCommand,
) -> Result<(), nocterm_cloud_model::tool::ToolExecutionError> {
    background::retain(driver, command)
}
pub(super) fn finish(
    driver: &mut Driver,
    command: &nocterm_cloud_model::tool::BackgroundCommand,
    outcome: nocterm_cloud_model::tool::BackgroundCommandOutcome,
) -> Result<(), nocterm_cloud_model::tool::ToolExecutionError> {
    background::finish(driver, command, outcome)
}

pub(super) fn require_original(
    db: &Connection,
    cipher: &dyn JournalCipher,
    stream: &StreamBinding,
    original: &nocterm_cloud_model::run::ToolIntent,
) -> Result<(), JournalError> {
    let stored = load(db, cipher, stream, original.id())?.ok_or(JournalError::Conflict)?;
    if stored.intent()? != *original {
        return Err(JournalError::Conflict);
    }
    Ok(())
}

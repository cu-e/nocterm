use super::{codec, head, integer, storage, unsigned};
use nocterm_cloud_model::{identity::*, journal::*, run::CommitStamp};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

pub(super) fn transition(
    db: &Connection,
    cipher: &dyn JournalCipher,
    stamp: &CommitStamp,
    sequence: JournalSequence,
    revision: StreamRevision,
) -> Result<(), JournalError> {
    let stream = stamp.run().stream();
    let run = key(stamp);
    let existing = read(db, cipher, stamp)?;
    let (epoch, previous) = existing.map_or((1, 0), |stamp| {
        (stamp.run().epoch().get(), stamp.revision().get())
    });
    if stamp.revision().get() != previous.checked_add(1).ok_or(JournalError::Capacity)?
        || !(epoch..=epoch.saturating_add(1)).contains(&stamp.run().epoch().get())
    {
        return Err(JournalError::Conflict);
    }
    let context = JournalCipherContext::new(
        stream.clone(),
        JournalStorageClass::RunHead,
        *stamp.run().run().as_bytes(),
        sequence,
        revision,
        stamp.writer(),
    );
    let body = codec::seal(cipher, &context, &codec::record::Stamp::from(stamp))?;
    db.execute("INSERT INTO run_heads(chat,run,epoch,revision,sequence,stream_revision,writer,body) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(chat,run) DO UPDATE SET epoch=excluded.epoch,revision=excluded.revision,sequence=excluded.sequence,stream_revision=excluded.stream_revision,writer=excluded.writer,body=excluded.body",params![head::key(&stream).as_slice(),run.as_slice(),integer(stamp.run().epoch().get())?,integer(stamp.revision().get())?,integer(sequence.get())?,integer(revision.get())?,integer(stamp.writer().get())?,body]).map_err(storage)?;
    Ok(())
}
fn read(
    db: &Connection,
    cipher: &dyn JournalCipher,
    next: &CommitStamp,
) -> Result<Option<CommitStamp>, JournalError> {
    let stream = next.run().stream();
    let run = key(next);
    let row=db.query_row("SELECT epoch,revision,sequence,stream_revision,writer,length(body) FROM run_heads WHERE chat=?1 AND run=?2",params![head::key(&stream).as_slice(),run.as_slice()],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?,row.get::<_,i64>(3)?,row.get::<_,i64>(4)?,row.get::<_,i64>(5)?))).optional().map_err(storage)?;
    let Some((epoch, run_revision, sequence, revision, writer, length)) = row else {
        return Ok(None);
    };
    if head::check_length(length)? > 4096 {
        return Err(JournalError::Corrupt);
    }
    let context = JournalCipherContext::new(
        stream.clone(),
        JournalStorageClass::RunHead,
        *next.run().run().as_bytes(),
        JournalSequence::from_u64(unsigned(sequence)?),
        StreamRevision::from_u64(unsigned(revision)?),
        WriterEpoch::new(unsigned(writer)?)?,
    );
    let body: Vec<u8> = db
        .query_row(
            "SELECT body FROM run_heads WHERE chat=?1 AND run=?2",
            params![head::key(&stream).as_slice(), run.as_slice()],
            |row| row.get(0),
        )
        .map_err(storage)?;
    let stamp = codec::open::<codec::record::Stamp>(cipher, &context, &body)?.restore()?;
    if !stamp.run().same_run(next.run())
        || stamp.run().epoch().get() != unsigned(epoch)?
        || stamp.revision().get() != unsigned(run_revision)?
        || stamp.writer() != context.writer()
    {
        return Err(JournalError::Corrupt);
    }
    Ok(Some(stamp))
}
fn key(stamp: &CommitStamp) -> [u8; 32] {
    Sha256::digest(stamp.run().run().as_bytes()).into()
}

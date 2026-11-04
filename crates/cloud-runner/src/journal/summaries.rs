//! Journal v7 summary rows: per chat the newest staged summary revision, the newest one the
//! hub confirmed, the creation and last-activity times and, while unconfirmed, the sealed
//! bytes of the staged revision (ADR 0135).
//!
//! The sealed bytes are already sealed to the account; the times are scheduling metadata
//! the hub sees anyway. Nothing here needs the chat key, so a deletion summary can be
//! staged after the key is retired. One bounded row per catalog chat; it is not part of the
//! chat's logical storage charge.
use super::{DurableRunJournal, head, integer, storage, unsigned};
use nocterm_cloud_model::{
    authority::StreamBinding,
    identity::{ChatKeyEpoch, Timestamp},
    journal::{
        ChatSummaryJournal, JournalError, JournalFuture, MAX_SEALED_CHAT_SUMMARY_BYTES,
        SealedChatSummary, StoredChatSummary,
    },
    payload::OpaqueBytes,
};
use rusqlite::{Connection, OptionalExtension, params};

/// The table journal v7 adds; `schema.sql` creates the same one.
pub(super) const TABLE: &str = "CREATE TABLE chat_summaries (
  chat BLOB PRIMARY KEY REFERENCES chats(chat),
  revision INTEGER NOT NULL CHECK(revision>0),
  published INTEGER NOT NULL CHECK(published>=0 AND published<=revision),
  created_at INTEGER NOT NULL CHECK(created_at>0),
  last_activity INTEGER NOT NULL CHECK(last_activity>=created_at),
  deleted INTEGER NOT NULL CHECK(deleted IN(0,1)),
  key_epoch INTEGER CHECK(key_epoch IS NULL OR key_epoch>0),
  sealed BLOB CHECK(sealed IS NULL OR length(sealed) BETWEEN 1 AND 4096),
  CHECK((sealed IS NULL)=(key_epoch IS NULL)),
  CHECK((published<revision)=(sealed IS NOT NULL))
) WITHOUT ROWID;";

impl ChatSummaryJournal for DurableRunJournal {
    fn summary(&self, stream: StreamBinding) -> JournalFuture<Option<StoredChatSummary>> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_driver(|driver| {
                driver.catalog.require_stream(&stream)?;
                load(&driver.connection, &stream)
            })
        })
    }
    fn stage_summary(
        &self,
        sealed: SealedChatSummary,
        created_at: Timestamp,
        last_activity: Timestamp,
    ) -> JournalFuture<()> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_driver(|driver| {
                driver.catalog.require_stream(sealed.stream())?;
                let tx = driver.connection.transaction().map_err(storage)?;
                stage(&tx, &sealed, created_at, last_activity)?;
                tx.commit().map_err(storage)
            })
        })
    }
    fn summary_published(&self, stream: StreamBinding, revision: u64) -> JournalFuture<()> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_driver(|driver| {
                driver.catalog.require_stream(&stream)?;
                driver
                    .connection
                    .execute(
                        "UPDATE chat_summaries SET published=?2,key_epoch=NULL,sealed=NULL WHERE chat=?1 AND revision=?2 AND published<?2",
                        params![head::key(&stream).as_slice(), integer(revision)?],
                    )
                    .map_err(storage)?;
                Ok(())
            })
        })
    }
}

fn load(
    db: &Connection,
    stream: &StreamBinding,
) -> Result<Option<StoredChatSummary>, JournalError> {
    let key = head::key(stream);
    let row = db
        .query_row(
            "SELECT revision,published,created_at,last_activity,deleted,key_epoch,length(sealed) FROM chat_summaries WHERE chat=?1",
            params![key.as_slice()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            },
        )
        .optional()
        .map_err(storage)?;
    let Some((revision, published, created, last, deleted, epoch, length)) = row else {
        return Ok(None);
    };
    let revision = unsigned(revision)?;
    let deleted = match deleted {
        0 => false,
        1 => true,
        _ => return Err(JournalError::Corrupt),
    };
    let pending = match (epoch, length) {
        (None, None) => None,
        (Some(epoch), Some(length)) => {
            let length = usize::try_from(length).map_err(|_| JournalError::Corrupt)?;
            if length == 0 || length > MAX_SEALED_CHAT_SUMMARY_BYTES {
                return Err(JournalError::Corrupt);
            }
            let sealed: Vec<u8> = db
                .query_row(
                    "SELECT sealed FROM chat_summaries WHERE chat=?1",
                    params![key.as_slice()],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            Some(
                SealedChatSummary::new(
                    stream.clone(),
                    ChatKeyEpoch::new(unsigned(epoch)?)?,
                    revision,
                    deleted,
                    OpaqueBytes::new(sealed)?,
                )
                .map_err(|_| JournalError::Corrupt)?,
            )
        }
        _ => return Err(JournalError::Corrupt),
    };
    StoredChatSummary::new(
        revision,
        unsigned(published)?,
        Timestamp::from_millis(unsigned(created)?),
        Timestamp::from_millis(unsigned(last)?),
        deleted,
        pending,
    )
    .map(Some)
}

fn stage(
    db: &Connection,
    sealed: &SealedChatSummary,
    created_at: Timestamp,
    last_activity: Timestamp,
) -> Result<(), JournalError> {
    if created_at.as_millis() == 0 || last_activity < created_at {
        return Err(JournalError::InvalidBatch);
    }
    let stored = load(db, sealed.stream())?;
    if stored
        .as_ref()
        .is_some_and(|stored| sealed.revision() <= stored.revision().max(stored.published()))
    {
        return Err(JournalError::Conflict);
    }
    let published = stored.as_ref().map_or(0, StoredChatSummary::published);
    db.execute(
        "INSERT INTO chat_summaries(chat,revision,published,created_at,last_activity,deleted,key_epoch,sealed) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(chat) DO UPDATE SET revision=excluded.revision,created_at=excluded.created_at,last_activity=excluded.last_activity,deleted=excluded.deleted,key_epoch=excluded.key_epoch,sealed=excluded.sealed",
        params![
            head::key(sealed.stream()).as_slice(),
            integer(sealed.revision())?,
            integer(published)?,
            integer(created_at.as_millis())?,
            integer(last_activity.as_millis())?,
            i64::from(sealed.deleted()),
            integer(sealed.key_epoch().get())?,
            sealed.ciphertext().as_bytes(),
        ],
    )
    .map_err(storage)?;
    Ok(())
}

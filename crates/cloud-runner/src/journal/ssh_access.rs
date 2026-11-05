//! Journal SSH rows (v8, sealed under their own class since v9): per chat its SSH binding,
//! open certificate request, active certificate and the seed of its Ed25519 subject key
//! (ADR 0145, 0147).
//!
//! The whole row is sealed with the chat's journal cipher under
//! [`JournalStorageClass::SshAccess`], so the seed is never stored in the clear; its key
//! epoch is kept beside it. A row sealed under an older chat key epoch
//! no longer opens and reads as no row: the binding named the old coordinate anyway.
use super::{DurableRunJournal, codec, head, integer, storage, unsigned};
use crate::codec::ssh::OfferRow;
use nocterm_cloud_model::{
    authority::StreamBinding, identity::*, journal::*, payload::OpaqueBytes, ssh_access::*,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// The table journal v8 adds; `schema.sql` creates the same one.
pub(super) const TABLE: &str = "CREATE TABLE chat_ssh_keys (
  chat BLOB PRIMARY KEY REFERENCES chats(chat),
  revision INTEGER NOT NULL CHECK(revision>0),
  key_epoch INTEGER NOT NULL CHECK(key_epoch>0),
  body BLOB NOT NULL CHECK(length(body) BETWEEN 1 AND 131072)
) WITHOUT ROWID;";

/// The cipher slot id of the row: the contract's `SSH_ACCESS_SLOT`, the one id of its class.
const SLOT: [u8; 16] = *b"nocterm-ssh-row1";

impl ChatSshKeys for DurableRunJournal {
    fn ssh_access(&self, stream: StreamBinding) -> JournalFuture<Option<StoredSshAccess>> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_driver(|driver| {
                driver.catalog.require_stream(&stream)?;
                load(&driver.connection, driver.cipher.as_ref(), &stream)
            })
        })
    }
    fn store_ssh_access(
        &self,
        stream: StreamBinding,
        stored: StoredSshAccess,
    ) -> JournalFuture<()> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_driver(|driver| {
                driver.catalog.require_stream(&stream)?;
                store(&driver.connection, driver.cipher.as_ref(), &stream, &stored)
            })
        })
    }
    fn ssh_streams(&self) -> JournalFuture<Vec<StreamBinding>> {
        let journal = self.clone();
        Box::pin(async move {
            journal.with_driver(|driver| {
                let mut query = driver
                    .connection
                    .prepare("SELECT c.chat_id FROM chat_ssh_keys s JOIN chats c ON c.chat=s.chat")
                    .map_err(storage)?;
                let ids = query
                    .query_map([], |row| row.get::<_, Vec<u8>>(0))
                    .map_err(storage)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(storage)?;
                ids.into_iter()
                    .map(|id| {
                        let id: [u8; 16] = id.try_into().map_err(|_| JournalError::Corrupt)?;
                        Ok(journal.binding().stream(CloudChatId::from_bytes(id)))
                    })
                    .collect()
            })
        })
    }
}

fn context(stream: &StreamBinding, revision: u64) -> Result<JournalCipherContext, JournalError> {
    Ok(JournalCipherContext::new(
        stream.clone(),
        JournalStorageClass::SshAccess,
        SLOT,
        JournalSequence::from_u64(revision),
        StreamRevision::from_u64(revision),
        WriterEpoch::new(1)?,
    ))
}

fn load(
    db: &Connection,
    cipher: &dyn JournalCipher,
    stream: &StreamBinding,
) -> Result<Option<StoredSshAccess>, JournalError> {
    let row = db
        .query_row(
            "SELECT revision,key_epoch,body FROM chat_ssh_keys WHERE chat=?1",
            params![head::key(stream).as_slice()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(storage)?;
    let Some((revision, epoch, body)) = row else {
        return Ok(None);
    };
    match cipher.chat_key_epoch(stream) {
        Ok(current) if current.get() == unsigned(epoch)? => {}
        _ => return Ok(None),
    }
    let row: Row = codec::open(cipher, &context(stream, unsigned(revision)?)?, &body)?;
    row.restore().map(Some)
}

fn store(
    db: &Connection,
    cipher: &dyn JournalCipher,
    stream: &StreamBinding,
    stored: &StoredSshAccess,
) -> Result<(), JournalError> {
    let key = head::key(stream);
    let previous: Option<i64> = db
        .query_row(
            "SELECT revision FROM chat_ssh_keys WHERE chat=?1",
            params![key.as_slice()],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?;
    let revision = previous
        .map_or(Ok(0), unsigned)?
        .checked_add(1)
        .ok_or(JournalError::Capacity)?;
    let epoch = cipher.chat_key_epoch(stream)?;
    let body = codec::seal(cipher, &context(stream, revision)?, &Row::from(stored))?;
    db.execute(
        "INSERT INTO chat_ssh_keys(chat,revision,key_epoch,body) VALUES(?1,?2,?3,?4) ON CONFLICT(chat) DO UPDATE SET revision=excluded.revision,key_epoch=excluded.key_epoch,body=excluded.body",
        params![key.as_slice(), integer(revision)?, integer(epoch.get())?, body],
    )
    .map_err(storage)?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Row {
    seed: Option<[u8; 32]>,
    epochs: Vec<([u8; 16], u64)>,
    binding: Option<BindingRow>,
}
impl Drop for Row {
    fn drop(&mut self) {
        if let Some(seed) = self.seed.as_mut() {
            seed.zeroize();
        }
    }
}
#[derive(Serialize, Deserialize)]
struct BindingRow {
    offer: OfferRow,
    reach: Vec<([u8; 16], bool)>,
    refused: Option<u8>,
    request: Option<RequestRow>,
    certificate: Option<CertificateRow>,
}
#[derive(Serialize, Deserialize)]
struct RequestRow {
    id: [u8; 16],
    servers: Vec<[u8; 16]>,
    at: u64,
    not_after: u64,
}
#[derive(Serialize, Deserialize)]
struct CertificateRow {
    request: [u8; 16],
    servers: Vec<[u8; 16]>,
    blob: Vec<u8>,
    window: (u64, u64),
}

fn servers(values: &[SshServerKey]) -> Vec<[u8; 16]> {
    values.iter().map(|server| *server.as_bytes()).collect()
}
fn server_keys(values: Vec<[u8; 16]>) -> Vec<SshServerKey> {
    values.into_iter().map(SshServerKey::from_bytes).collect()
}
fn at(value: u64) -> Timestamp {
    Timestamp::from_millis(value)
}

impl From<&StoredSshAccess> for Row {
    fn from(stored: &StoredSshAccess) -> Self {
        let parts = &stored.access;
        Self {
            seed: stored.seed.as_ref().map(|seed| *seed.expose()),
            epochs: parts
                .epochs
                .iter()
                .map(|(id, epoch)| (*id.as_bytes(), epoch.get()))
                .collect(),
            binding: parts.binding.as_ref().map(|binding| BindingRow {
                offer: OfferRow::from(&binding.offer),
                reach: binding
                    .reach
                    .iter()
                    .map(|(server, reach)| (*server.as_bytes(), *reach == SshServerReach::Admitted))
                    .collect(),
                refused: binding.refused.map(|refusal| match refusal {
                    SshBindingRefusal::NoSourceAddress => 1,
                    SshBindingRefusal::NoReachableServer => 2,
                    SshBindingRefusal::Invalid => 3,
                }),
                request: binding.request.as_ref().map(|open| RequestRow {
                    id: open.id,
                    servers: servers(&open.servers),
                    at: open.at.as_millis(),
                    not_after: open.not_after.as_millis(),
                }),
                certificate: binding.certificate.as_ref().map(|issued| CertificateRow {
                    request: issued.request,
                    servers: servers(&issued.servers),
                    blob: issued.blob.as_bytes().to_vec(),
                    window: (
                        issued.valid_after.as_millis(),
                        issued.valid_before.as_millis(),
                    ),
                }),
            }),
        }
    }
}
impl Row {
    fn restore(mut self) -> Result<StoredSshAccess, JournalError> {
        let seed = self.seed.take().map(SshSubjectSeed::new);
        let epochs = std::mem::take(&mut self.epochs)
            .into_iter()
            .map(|(id, epoch)| Ok((GrantId::from_bytes(id), GrantEpoch::new(epoch)?)))
            .collect::<Result<_, JournalError>>()?;
        let binding = self.binding.take().map(BindingRow::restore).transpose()?;
        let access = SshAccessParts { epochs, binding };
        // Only parts a transition could produce are restored.
        ChatSshAccess::restore(access.clone()).map_err(|_| JournalError::Corrupt)?;
        if seed.is_none() && access.binding.as_ref().is_some_and(|b| b.refused.is_none()) {
            return Err(JournalError::Corrupt);
        }
        Ok(StoredSshAccess { seed, access })
    }
}
impl BindingRow {
    fn restore(self) -> Result<BindingParts, JournalError> {
        let offer = self.offer.restore()?;
        let refused = self
            .refused
            .map(|tag| match tag {
                1 => Ok(SshBindingRefusal::NoSourceAddress),
                2 => Ok(SshBindingRefusal::NoReachableServer),
                3 => Ok(SshBindingRefusal::Invalid),
                _ => Err(JournalError::Corrupt),
            })
            .transpose()?;
        let certificate = self
            .certificate
            .map(|row| {
                Ok::<_, JournalError>(IssuedCertificate {
                    request: row.request,
                    servers: server_keys(row.servers),
                    blob: OpaqueBytes::new(row.blob).map_err(|_| JournalError::Corrupt)?,
                    valid_after: at(row.window.0),
                    valid_before: at(row.window.1),
                })
            })
            .transpose()?;
        Ok(BindingParts {
            offer,
            reach: self
                .reach
                .into_iter()
                .map(|(server, admitted)| {
                    let reach = if admitted {
                        SshServerReach::Admitted
                    } else {
                        SshServerReach::Unreachable
                    };
                    (SshServerKey::from_bytes(server), reach)
                })
                .collect(),
            refused,
            request: self.request.map(|open| OpenRequest {
                id: open.id,
                servers: server_keys(open.servers),
                at: at(open.at),
                not_after: at(open.not_after),
            }),
            certificate,
        })
    }
}

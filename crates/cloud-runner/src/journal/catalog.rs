//! Authenticated membership and ordered current-head commitment for one runner.
mod leaves;
use super::{head, integer, metadata, storage, unsigned};
use crate::codec::identity::Scope;
use nocterm_cloud_model::{identity::*, journal::*};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::Arc};
use zeroize::Zeroizing;

const FORMAT: u8 = 2;

#[derive(Serialize, Deserialize)]
struct StoredCatalog {
    scope: Scope,
    runner: [u8; 16],
    epoch: u64,
    revision: u64,
    chats: ChatIds,
    head_root: [u8; 32],
}
#[derive(Serialize)]
struct ChatIds(Vec<[u8; 16]>);
impl<'de> Deserialize<'de> for ChatIds {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = ChatIds;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a bounded catalog of chat identities")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut chats = Vec::new();
                while let Some(chat) = sequence.next_element()? {
                    if chats.len() == MAX_JOURNAL_CATALOG_CHATS {
                        return Err(serde::de::Error::custom("catalog entry capacity"));
                    }
                    chats.push(chat);
                }
                Ok(ChatIds(chats))
            }
        }
        decoder.deserialize_seq(Visitor)
    }
}

pub(super) struct Catalog {
    revision: u64,
    chats: BTreeSet<[u8; 16]>,
    hash: [u8; 32],
    head_root: [u8; 32],
}
impl Catalog {
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }
    pub(super) fn hash(&self) -> Fingerprint {
        Fingerprint::from_bytes(self.hash)
    }
    pub(super) fn chats(&self) -> impl Iterator<Item = CloudChatId> + '_ {
        self.chats.iter().copied().map(CloudChatId::from_bytes)
    }
    pub(super) fn len(&self) -> usize {
        self.chats.len()
    }
    pub(super) fn advance(&mut self) -> Result<(), JournalError> {
        self.revision = self.revision.checked_add(1).ok_or(JournalError::Capacity)?;
        Ok(())
    }
    pub(super) fn streams<'a>(
        &'a self,
        binding: &'a JournalCatalogBinding,
    ) -> impl Iterator<Item = crate::journal::StreamBinding> + 'a {
        self.chats
            .iter()
            .map(|id| binding.stream(CloudChatId::from_bytes(*id)))
    }
    pub(super) fn insert(&mut self, chat: CloudChatId) -> Result<bool, JournalError> {
        if self.chats.contains(chat.as_bytes()) {
            return Ok(false);
        }
        if self.chats.len() == MAX_JOURNAL_CATALOG_CHATS {
            return Err(JournalError::Capacity);
        }
        self.chats.insert(*chat.as_bytes());
        Ok(true)
    }
}

/// The cached mark is a refusal fence, not a second authoritative catalog.
#[derive(Clone)]
pub(super) struct CatalogOwner {
    binding: JournalCatalogBinding,
    observed: Arc<Mutex<CatalogMark>>,
    chats: u64,
    bytes: usize,
}
impl CatalogOwner {
    pub(super) fn new(binding: JournalCatalogBinding, chats: u64, bytes: usize) -> Self {
        Self {
            binding,
            observed: Arc::new(Mutex::new(CatalogMark::default())),
            chats,
            bytes,
        }
    }
    pub(super) fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub(super) fn require_stream(
        &self,
        stream: &crate::journal::StreamBinding,
    ) -> Result<(), JournalError> {
        if self.binding.contains(stream) {
            Ok(())
        } else {
            Err(JournalError::Fenced)
        }
    }
    pub(super) fn create(
        &mut self,
        db: &mut Connection,
        cipher: &dyn JournalCatalogCipher,
    ) -> Result<(), JournalError> {
        let tx = db.transaction().map_err(storage)?;
        let mut catalog = Catalog {
            revision: 1,
            chats: BTreeSet::new(),
            hash: [0; 32],
            head_root: [0; 32],
        };
        self.save(&tx, cipher, &mut catalog)?;
        tx.commit().map_err(storage)?;
        self.observe(&catalog)?;
        Ok(())
    }
    pub(super) fn load(
        &self,
        db: &Connection,
        cipher: &dyn JournalCatalogCipher,
    ) -> Result<Catalog, JournalError> {
        metadata::require_transaction(db)?;
        let cap = (self.bytes + 64 * 1024).min(MAX_JOURNAL_CATALOG_CIPHERTEXT);
        metadata::catalog(db, cap)?;
        metadata::rows(db, self.chats)?;
        // The type/length passes completed in this same snapshot before this
        // raw column access. substr would materialize the full source first.
        let (revision, sealed) = db
            .query_row(
                "SELECT revision,body FROM account_catalog WHERE singleton=1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()
            .map_err(storage)?
            .ok_or(JournalError::Corrupt)?;
        let revision = unsigned(revision)?;
        let hash: [u8; 32] = Sha256::digest(&sealed).into();
        let observed = *self.observed.lock();
        if revision < observed.revision || (revision == observed.revision && hash != observed.hash)
        {
            return Err(JournalError::Corrupt);
        }
        let context = JournalCatalogContext::new(self.binding.clone(), revision)?;
        let plain = Zeroizing::new(cipher.open(&context, &sealed)?);
        if plain.is_empty()
            || plain.len() > self.bytes
            || plain.len() > MAX_JOURNAL_CATALOG_PLAINTEXT
        {
            return Err(JournalError::Corrupt);
        }
        let (format, _) =
            postcard::take_from_bytes::<u8>(&plain).map_err(|_| JournalError::Corrupt)?;
        if format != FORMAT {
            return Err(JournalError::UnsupportedFormat);
        }
        let ((format, value), trailing) = postcard::take_from_bytes::<(u8, StoredCatalog)>(&plain)
            .map_err(|_| JournalError::Corrupt)?;
        if format != FORMAT
            || !trailing.is_empty()
            || value.revision != revision
            || value.scope.restore()? != *self.binding.scope()
            || value.runner != *self.binding.runner().as_bytes()
            || value.epoch != self.binding.epoch().get()
            || value.chats.0.len() as u64 > self.chats
        {
            return Err(JournalError::Corrupt);
        }
        let count = value.chats.0.len();
        let chats: BTreeSet<_> = value.chats.0.into_iter().collect();
        if chats.len() != count {
            return Err(JournalError::Corrupt);
        }
        Ok(Catalog {
            revision,
            chats,
            hash,
            head_root: value.head_root,
        })
    }
    pub(super) fn read_checked(
        &self,
        db: &Connection,
        cipher: &dyn JournalCatalogCipher,
    ) -> Result<Catalog, JournalError> {
        let tx = db.unchecked_transaction().map_err(storage)?;
        let catalog = self.load(&tx, cipher)?;
        self.verify_rows(&tx, &catalog)?;
        tx.commit().map_err(storage)?;
        Ok(catalog)
    }
    pub(super) fn verify_rows(
        &self,
        db: &Connection,
        catalog: &Catalog,
    ) -> Result<(), JournalError> {
        if leaves::root(db, &self.binding, &catalog.chats, self.chats)? != catalog.head_root {
            return Err(JournalError::Corrupt);
        }
        Ok(())
    }
    pub(super) fn save(
        &self,
        db: &Connection,
        cipher: &dyn JournalCatalogCipher,
        catalog: &mut Catalog,
    ) -> Result<(), JournalError> {
        if catalog.chats.len() as u64 > self.chats {
            return Err(JournalError::Capacity);
        }
        if catalog
            .chats
            .len()
            .checked_mul(16)
            .is_none_or(|bytes| bytes > self.bytes)
        {
            return Err(JournalError::Capacity);
        }
        catalog.head_root = leaves::root(db, &self.binding, &catalog.chats, self.chats)?;
        let value = StoredCatalog {
            scope: self.binding.scope().into(),
            runner: *self.binding.runner().as_bytes(),
            epoch: self.binding.epoch().get(),
            revision: catalog.revision,
            chats: ChatIds(catalog.chats.iter().copied().collect()),
            head_root: catalog.head_root,
        };
        let plain = Zeroizing::new(
            postcard::to_allocvec(&(FORMAT, value)).map_err(|_| JournalError::Corrupt)?,
        );
        if plain.len() > self.bytes || plain.len() > MAX_JOURNAL_CATALOG_PLAINTEXT {
            return Err(JournalError::Capacity);
        }
        let context = JournalCatalogContext::new(self.binding.clone(), catalog.revision)?;
        let sealed = cipher.seal(&context, &plain)?;
        if sealed.is_empty()
            || sealed.len() > self.bytes + 64 * 1024
            || sealed.len() > MAX_JOURNAL_CATALOG_CIPHERTEXT
        {
            return Err(JournalError::Capacity);
        }
        catalog.hash = Sha256::digest(&sealed).into();
        db.execute("INSERT INTO account_catalog(singleton,revision,body) VALUES(1,?1,?2) ON CONFLICT(singleton) DO UPDATE SET revision=excluded.revision,body=excluded.body", params![integer(catalog.revision)?,sealed]).map_err(storage)?;
        Ok(())
    }
    pub(super) fn observe(&self, catalog: &Catalog) -> Result<(), JournalError> {
        let mut observed = self.observed.lock();
        if catalog.revision < observed.revision
            || (catalog.revision == observed.revision && catalog.hash != observed.hash)
        {
            return Err(JournalError::Corrupt);
        }
        *observed = CatalogMark {
            revision: catalog.revision,
            hash: catalog.hash,
        };
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
struct CatalogMark {
    revision: u64,
    hash: [u8; 32],
}

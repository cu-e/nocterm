use super::{Driver, codec, integer, storage, unsigned};
use crate::storage::StorageBudget;
use nocterm_cloud_model::{authority::StreamBinding, identity::*, journal::*};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

pub(super) struct Head {
    lease: WriterLease,
    position: JournalPosition,
    retained: JournalSequence,
    deleted: bool,
    charge: StorageBudget,
    active: Vec<[u8; 16]>,
    acknowledged: EventSequence,
    event_floor: EventSequence,
    transfer: Option<[u8; 32]>,
}
impl Head {
    pub(super) fn lease(&self) -> &WriterLease {
        &self.lease
    }
    pub(super) fn position(&self) -> JournalPosition {
        self.position
    }
    pub(super) fn retained(&self) -> JournalSequence {
        self.retained
    }
    pub(super) fn deleted(&self) -> bool {
        self.deleted
    }
    pub(super) fn budget(&self) -> StorageBudget {
        self.charge
    }
    pub(super) fn acknowledged(&self) -> EventSequence {
        self.acknowledged
    }
    pub(super) fn event_floor(&self) -> EventSequence {
        self.event_floor
    }
    pub(super) fn acknowledge(&mut self, through: EventSequence) {
        self.acknowledged = self.acknowledged.max(through);
    }
    pub(super) fn active(&self) -> &[[u8; 16]] {
        &self.active
    }
    pub(super) fn set_active(&mut self, active: Vec<[u8; 16]>) {
        self.active = active;
    }
    pub(super) fn claim_cleanup(&mut self, claim: &WriterClaim) -> Result<(), JournalError> {
        if !self.deleted
            || self.lease.stream() != claim.stream()
            || claim.expected() != Some(self.lease.epoch())
            || claim.next() != self.lease.epoch().next()?
        {
            return Err(JournalError::Fenced);
        }
        self.lease = WriterLease::new(claim.stream().clone(), claim.next());
        Ok(())
    }
    pub(super) fn require_cleanup(&self, lease: &CleanupLease) -> Result<(), JournalError> {
        if &self.lease != lease.writer() {
            return Err(JournalError::Fenced);
        }
        Ok(())
    }
    pub(super) fn require_writer(&self, lease: &WriterLease) -> Result<(), JournalError> {
        if self.lease != *lease || self.deleted {
            Err(JournalError::Fenced)
        } else {
            Ok(())
        }
    }
    pub(super) fn update(
        &mut self,
        position: JournalPosition,
        deleted: bool,
        charge: StorageBudget,
    ) {
        self.position = position;
        self.deleted = deleted;
        self.charge = charge;
    }
    pub(super) fn compacted(&mut self, retained: JournalSequence, floor: EventSequence) {
        self.retained = retained;
        self.event_floor = floor;
    }
    pub(super) fn transfer(&self) -> Option<[u8; 32]> {
        self.transfer
    }
    pub(super) fn set_transfer(&mut self, digest: [u8; 32]) {
        self.transfer = Some(digest);
    }
    pub(super) fn save(
        &self,
        db: &Connection,
        cipher: &dyn JournalCipher,
    ) -> Result<(), JournalError> {
        let context = context(
            self.lease.stream(),
            JournalStorageClass::Header,
            [0; 16],
            self.position,
            self.lease.epoch(),
        );
        let body = codec::seal(
            cipher,
            &context,
            &codec::Header::new(
                self.lease.stream(),
                self.lease.epoch(),
                self.position,
                self.retained,
                self.deleted,
                self.charge,
                self.active.clone(),
                self.acknowledged,
                self.event_floor,
                self.transfer,
            ),
        )?;
        if body.is_empty() || body.len() > super::header_link::MAX_HEADER_CIPHERTEXT {
            return Err(JournalError::Capacity);
        }
        let digest: [u8; 32] = Sha256::digest(&body).into();
        let stream = self.lease.stream();
        db.execute("INSERT INTO chats(chat,binding,scope,chat_id,runner,key_epoch,writer,revision,records,events,control,retained,deleted,charge,body,header_digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16) ON CONFLICT(chat) DO UPDATE SET writer=excluded.writer,revision=excluded.revision,records=excluded.records,events=excluded.events,control=excluded.control,retained=excluded.retained,deleted=excluded.deleted,charge=excluded.charge,body=excluded.body,header_digest=excluded.header_digest", params![key(stream).as_slice(), binding_key(stream).as_slice(), scope_key(stream.scope()).as_slice(), stream.chat().as_bytes().as_slice(), stream.runner().as_bytes().as_slice(), integer(stream.runner_epoch().get())?, integer(self.lease.epoch().get())?, integer(self.position.revision().get())?, integer(self.position.records().get())?, integer(self.position.events().get())?, integer(self.position.control().get())?, integer(self.retained.get())?, self.deleted, integer(self.charge.total())?, body, digest.as_slice()]).map_err(storage)?;
        Ok(())
    }
}

pub(super) fn claim(driver: &mut Driver, claim: &WriterClaim) -> Result<WriterLease, JournalError> {
    driver.database.validate_leaves()?;
    driver.catalog.require_stream(claim.stream())?;
    let db = driver
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let mut catalog = driver.catalog.load(&db, driver.catalog_cipher.as_ref())?;
    driver.catalog.verify_rows(&db, &catalog)?;
    let previous = driver.heads.load(&db, claim.stream())?;
    let existing = load(&db, driver.cipher.as_ref(), claim.stream())?;
    let mut head = match existing {
        Some(head) => {
            if head.deleted || claim.expected() != Some(head.lease.epoch()) {
                return Err(JournalError::Fenced);
            }
            head
        }
        None => {
            if claim.expected().is_some() {
                return Err(JournalError::Fenced);
            }
            if catalog.len() as u64 >= driver.limits.chats {
                return Err(JournalError::Capacity);
            }
            Head {
                lease: WriterLease::new(claim.stream().clone(), claim.next()),
                position: JournalPosition::default(),
                retained: JournalSequence::default(),
                deleted: false,
                charge: StorageBudget::default(),
                active: Vec::new(),
                acknowledged: EventSequence::default(),
                event_floor: EventSequence::default(),
                transfer: None,
            }
        }
    };
    head.lease = WriterLease::new(claim.stream().clone(), claim.next());
    head.save(&db, driver.cipher.as_ref())?;
    catalog.insert(claim.stream().chat())?;
    driver
        .heads
        .publication(
            driver.cipher.as_ref(),
            &driver.catalog,
            driver.catalog_cipher.as_ref(),
        )
        .publish(&db, &mut catalog, &head, previous.as_ref(), None)?;
    db.commit().map_err(storage)?;
    driver.catalog.observe(&catalog)?;
    Ok(head.lease)
}

pub(super) fn load(
    db: &Connection,
    cipher: &dyn JournalCipher,
    stream: &StreamBinding,
) -> Result<Option<Head>, JournalError> {
    let row = db.query_row("SELECT binding,writer,revision,records,events,control,retained,deleted,charge,length(body) FROM chats WHERE chat=?1", [key(stream).as_slice()], |row| {
        Ok((row.get::<_,Vec<u8>>(0)?, row.get::<_,i64>(1)?, row.get::<_,i64>(2)?, row.get::<_,i64>(3)?, row.get::<_,i64>(4)?, row.get::<_,i64>(5)?, row.get::<_,i64>(6)?, row.get::<_,bool>(7)?, row.get::<_,i64>(8)?, row.get::<_,i64>(9)?))
    }).optional().map_err(storage)?;
    let Some((
        binding,
        writer,
        revision,
        records,
        events,
        control,
        retained,
        deleted,
        charge,
        length,
    )) = row
    else {
        return Ok(None);
    };
    if binding != binding_key(stream) {
        return Err(JournalError::Fenced);
    }
    check_length(length)?;
    let epoch = WriterEpoch::new(unsigned(writer)?)?;
    let position = JournalPosition::new(
        StreamRevision::from_u64(unsigned(revision)?),
        ChatControlRevision::from_u64(unsigned(control)?),
        JournalSequence::from_u64(unsigned(records)?),
        EventSequence::from_u64(unsigned(events)?),
    );
    let context = context(
        stream,
        JournalStorageClass::Header,
        [0; 16],
        position,
        epoch,
    );
    let body: Vec<u8> = db
        .query_row(
            "SELECT body FROM chats WHERE chat=?1",
            [key(stream).as_slice()],
            |row| row.get(0),
        )
        .map_err(storage)?;
    let restored = codec::open::<codec::Header>(cipher, &context, &body)?.restore()?;
    if restored.0 != *stream
        || restored.1 != epoch
        || restored.2 != position
        || restored.3 != JournalSequence::from_u64(unsigned(retained)?)
        || restored.4 != deleted
        || restored.5.total() != unsigned(charge)?
    {
        return Err(JournalError::Corrupt);
    }
    Ok(Some(Head {
        lease: WriterLease::new(stream.clone(), epoch),
        position,
        retained: restored.3,
        deleted,
        charge: restored.5,
        active: restored.6,
        acknowledged: restored.7,
        event_floor: restored.8,
        transfer: restored.9,
    }))
}

pub(super) fn required(
    db: &Connection,
    cipher: &dyn JournalCipher,
    stream: &StreamBinding,
) -> Result<Head, JournalError> {
    load(db, cipher, stream)?.ok_or(JournalError::NotFound)
}
pub(super) fn context(
    stream: &StreamBinding,
    class: JournalStorageClass,
    id: [u8; 16],
    position: JournalPosition,
    writer: WriterEpoch,
) -> JournalCipherContext {
    JournalCipherContext::new(
        stream.clone(),
        class,
        id,
        position.records(),
        position.revision(),
        writer,
    )
}
pub(super) fn check_length(length: i64) -> Result<usize, JournalError> {
    let length = usize::try_from(length).map_err(|_| JournalError::Corrupt)?;
    if length == 0 || length > MAX_STORED_JOURNAL_BYTES {
        return Err(JournalError::Corrupt);
    }
    Ok(length)
}
pub(super) fn key(stream: &StreamBinding) -> [u8; 32] {
    let scope = stream.scope();
    let mut hash = Sha256::new();
    hash.update(b"nocterm/native-cloud-chat/v1");
    for value in [
        scope.hub().instance().as_str().as_bytes(),
        scope.hub().authority().as_bytes(),
        scope.account().id().as_str().as_bytes(),
        scope.account().key().as_bytes(),
        stream.chat().as_bytes(),
    ] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value);
    }
    hash.finalize().into()
}
pub(super) fn binding_key(stream: &StreamBinding) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(key(stream));
    hash.update(stream.runner().as_bytes());
    hash.update(stream.runner_epoch().get().to_be_bytes());
    hash.finalize().into()
}
pub(super) fn scope_key(scope: &AccountScope) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"nocterm/native-cloud-scope/v1");
    for value in [
        scope.hub().instance().as_str().as_bytes(),
        scope.hub().authority().as_bytes(),
        scope.account().id().as_str().as_bytes(),
        scope.account().key().as_bytes(),
    ] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value);
    }
    hash.finalize().into()
}

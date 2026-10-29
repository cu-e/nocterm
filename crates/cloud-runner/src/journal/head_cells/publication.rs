//! One root-cell/catalog publication inside its caller's SQL transaction.
use super::*;
use crate::journal::catalog::Catalog;

pub(in crate::journal) struct HeadPublication<'a> {
    heads: &'a HeadCells,
    content: &'a dyn JournalCipher,
    catalog_owner: &'a CatalogOwner,
    catalog_cipher: &'a dyn JournalCatalogCipher,
}
impl<'a> HeadPublication<'a> {
    pub(in crate::journal) fn new(
        heads: &'a HeadCells,
        content: &'a dyn JournalCipher,
        catalog_owner: &'a CatalogOwner,
        catalog_cipher: &'a dyn JournalCatalogCipher,
    ) -> Self {
        Self {
            heads,
            content,
            catalog_owner,
            catalog_cipher,
        }
    }
    pub(in crate::journal) fn publish(
        &self,
        db: &Connection,
        catalog: &mut Catalog,
        current: &head::Head,
        old: Option<&Cell>,
        published: Option<&OutboxEntry>,
    ) -> Result<(), JournalError> {
        self.publish_with(db, catalog, current, old, published, None)
    }
    pub(in crate::journal) fn publish_baseline(
        &self,
        db: &Connection,
        catalog: &mut Catalog,
        current: &head::Head,
        old: &Cell,
        baseline: &HeadAnchor,
    ) -> Result<(), JournalError> {
        self.publish_with(db, catalog, current, Some(old), None, Some(baseline))
    }
    fn publish_with(
        &self,
        db: &Connection,
        catalog: &mut Catalog,
        current: &head::Head,
        old: Option<&Cell>,
        published: Option<&OutboxEntry>,
        baseline: Option<&HeadAnchor>,
    ) -> Result<(), JournalError> {
        let stream = current.lease().stream();
        let material = self.content.chat_key_epoch(stream)?;
        if old.is_some_and(|old| old.value.key_epoch() != material) {
            // A new installed key is not an activated journal generation. Its
            // real signed Snapshot baseline must be published before the flip.
            return Err(JournalError::UnsupportedFormat);
        }
        if let Some(event) = published {
            let anchor = event.anchor();
            let position = anchor.position();
            if event.key_epoch() != material
                || position.writer() != current.lease().epoch()
                || position.event() != current.position().events()
                || position.journal() > current.position().records()
                || position.revision() != current.position().revision()
                || anchor.run().is_some_and(|run| run.stream() != *stream)
                || anchor.ciphertext().as_bytes().as_slice()
                    != Sha256::digest(event.sealed().as_bytes()).as_slice()
            {
                return Err(JournalError::InvalidBatch);
            }
        }
        let previous = old.map(|old| old.value.facts());
        let anchor = published
            .map(|entry| entry.anchor().clone())
            .or_else(|| previous.and_then(|facts| facts.head.clone()));
        let facts = CurrentHeadFacts {
            writer: Some(current.lease().epoch()),
            control: current.position().control(),
            deleted: current.deleted(),
            event_floor: if let Some(baseline) = baseline {
                baseline.position().event()
            } else if anchor.is_some() {
                previous
                    .map(|facts| facts.event_floor)
                    .filter(|floor| floor.get() > 0)
                    .unwrap_or(EventSequence::from_u64(1))
            } else {
                EventSequence::default()
            },
            head: anchor,
            snapshot: baseline
                .cloned()
                .or_else(|| previous.and_then(|facts| facts.snapshot.clone())),
        };
        let value = CurrentChatHead::new(stream.clone(), material, facts)?;
        let header = HeaderLink::read(db, stream)?;
        header.check_observation(&value)?;
        if let Some(old) = old {
            self.heads.check_progress(old, &value)?;
            if equal_facts(old.value.facts(), value.facts()) && old.header == header {
                return Ok(());
            }
        }
        let revision = old.map_or(Ok(1), |old| {
            old.revision.checked_add(1).ok_or(JournalError::Capacity)
        })?;
        self.heads.store(db, revision, &value, &header)?;
        catalog.advance()?;
        self.catalog_owner.save(db, self.catalog_cipher, catalog)
    }
}

//! A fresh signed page and its continuation remain bound to one captured account/R.
use super::source::{CatalogSource, PageStamp};
use super::{
    RunnerCatalogError, require, retained, runner_catalog_page_bytes, runner_catalog_work_bytes,
    sdk,
};
use crate::cloud::head_mapping::PinnedHeadMapper;
use nocterm_cloud_model::{
    identity::Fingerprint,
    journal::{CurrentChatHead, JournalCatalogBinding},
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use nocterm_hub_model::cloud::head::{
    CatalogHeadCursor, CatalogHeadPage, HeadTarget, HeadValue, MAX_HEAD_CATALOG_CHATS,
    MAX_HEAD_CATALOG_PAGE,
};
use std::sync::Arc;
use tokio::sync::OwnedSemaphorePermit;

#[derive(Clone)]
pub(super) struct Continuation {
    cursor: CatalogHeadCursor,
    total: u32,
}
struct PageData {
    source: Arc<CatalogSource>,
    entries: Vec<CurrentChatHead>,
    stamp: PageStamp,
    next: Option<Continuation>,
}
pub(super) fn native_owner_bytes() -> usize {
    std::mem::size_of::<PageData>()
}

/// Authenticated snapshot metadata, not execution authority. Borrowed rows and
/// the private continuation retain this exact native resident/source owner.
pub struct RunnerCatalogPage {
    resident: Allocated<PageData>,
}
impl RunnerCatalogPage {
    pub fn binding(&self) -> &JournalCatalogBinding {
        self.resident.value().source.binding()
    }
    pub fn revision(&self) -> u64 {
        self.resident.value().stamp.revision
    }
    pub fn digest(&self) -> Fingerprint {
        self.resident.value().stamp.root
    }
    pub fn total_chats(&self) -> u32 {
        self.resident.value().stamp.total
    }
    pub fn has_more(&self) -> bool {
        self.resident.value().next.is_some()
    }
    pub fn entries(&self) -> Result<&[CurrentChatHead], RunnerCatalogError> {
        self.resident.value().source.check()?;
        Ok(&self.resident.value().entries)
    }
    pub(super) fn continuation(
        &self,
        source: &Arc<CatalogSource>,
    ) -> Result<Continuation, RunnerCatalogError> {
        let data = self.resident.value();
        if !Arc::ptr_eq(source, &data.source) {
            return Err(RunnerCatalogError::Binding);
        }
        source.check()?;
        data.next.clone().ok_or(RunnerCatalogError::Complete)
    }
}
pub(super) struct CatalogOperation {
    source: Arc<CatalogSource>,
    sequence: PageSequence,
    resident: AllocationLease,
    work: AllocationLease,
    _turn: OwnedSemaphorePermit,
}
impl CatalogOperation {
    pub(super) fn new(
        source: Arc<CatalogSource>,
        after: Option<Continuation>,
        work: AllocationLease,
        resident: AllocationLease,
    ) -> Result<Self, RunnerCatalogError> {
        require(&work, runner_catalog_work_bytes())?;
        require(&resident, runner_catalog_page_bytes())?;
        source.check()?;
        let turn = source.turn()?;
        Ok(Self {
            source,
            sequence: PageSequence::new(after),
            resident,
            work,
            _turn: turn,
        })
    }
    pub(super) async fn execute(self) -> CatalogOutput {
        let result = self
            .read()
            .await
            .map_err(|error| retained(error, self.work.clone()));
        CatalogOutput {
            result,
            _work: self.work.clone(),
        }
    }
    async fn read(&self) -> Result<RunnerCatalogPage, RunnerCatalogError> {
        self.source.check()?;
        let port = self
            .source
            .cloud()
            .head_probes_with_residency(Arc::new(self.work.clone()))
            .map_err(sdk)?;
        let scope = self.source.scope()?;
        let target = self.sequence.target();
        let now = self.source.now()?;
        let prepared = self
            .source
            .with_keys(|keys| port.prepare(keys, scope, target, now))?;
        self.source.check()?;
        let completed = port.request(prepared).await.map_err(sdk)?;
        self.source.check()?;
        let now = self.source.now()?;
        let witness = self
            .source
            .with_keys(|keys| completed.open_resident(keys, now))?;
        self.source.check()?;
        let HeadValue::Catalog(page) = &witness.witness().witness().value else {
            return Err(RunnerCatalogError::Format);
        };
        let stamp = self.sequence.validate(page)?;
        let mapper = PinnedHeadMapper::new(self.source.binding().clone());
        let mut entries = Vec::with_capacity(page.chats.len());
        for value in &page.chats {
            entries.push(mapper.head(value).map_err(|_| RunnerCatalogError::Format)?);
        }
        let data = PageData {
            source: self.source.clone(),
            entries,
            stamp,
            next: PageSequence::continuation(page),
        };
        let result = RunnerCatalogPage {
            resident: Allocated::new(data, self.resident.clone()),
        };
        drop(witness);
        self.source.publish(stamp, || result)
    }
}

/// One opaque original-page continuation; no account, SDK session or network fake.
pub(super) struct PageSequence {
    after: Option<Continuation>,
}
impl PageSequence {
    pub(super) fn new(after: Option<Continuation>) -> Self {
        Self { after }
    }
    pub(super) fn target(&self) -> HeadTarget {
        HeadTarget::Catalog {
            after: self.after.as_ref().map(|after| after.cursor.clone()),
        }
    }
    pub(super) fn continuation(page: &CatalogHeadPage) -> Option<Continuation> {
        page.next_cursor().map(|cursor| Continuation {
            cursor,
            total: page.total_chats,
        })
    }
    pub(super) fn validate(&self, page: &CatalogHeadPage) -> Result<PageStamp, RunnerCatalogError> {
        let expected = self.after.as_ref().map(|after| &after.cursor);
        if page.revision == 0
            || page.chats.len() > MAX_HEAD_CATALOG_PAGE
            || page.total_chats > MAX_HEAD_CATALOG_CHATS
            || page.after.as_ref() != expected
        {
            return Err(RunnerCatalogError::Format);
        }
        if let Some(after) = &self.after
            && (after.cursor.revision != page.revision
                || after.cursor.digest != page.digest
                || after.total != page.total_chats)
        {
            return Err(RunnerCatalogError::Revision);
        }
        let seen = expected.map_or(0, |cursor| cursor.seen);
        let through = seen
            .checked_add(u32::try_from(page.chats.len()).map_err(|_| RunnerCatalogError::Capacity)?)
            .ok_or(RunnerCatalogError::Capacity)?;
        if through > page.total_chats
            || page.has_more != (through < page.total_chats)
            || (page.has_more && page.chats.is_empty())
        {
            return Err(RunnerCatalogError::Format);
        }
        let mut previous = expected.map(|cursor| cursor.after);
        for head in &page.chats {
            if previous.is_some_and(|id| head.chat_id <= id) {
                return Err(RunnerCatalogError::Format);
            }
            previous = Some(head.chat_id);
        }
        if page.has_more
            && page
                .next_cursor()
                .is_none_or(|next| next.seen <= seen || next.after != previous.unwrap_or_default())
        {
            return Err(RunnerCatalogError::Format);
        }
        Ok(PageStamp {
            revision: page.revision,
            root: Fingerprint::from_bytes(page.digest),
            total: page.total_chats,
        })
    }
}
/// An unobserved task completion keeps its original work through actual output Drop.
pub(super) struct CatalogOutput {
    result: Result<RunnerCatalogPage, RunnerCatalogError>,
    _work: AllocationLease,
}
impl CatalogOutput {
    pub(super) fn into_result(self) -> Result<RunnerCatalogPage, RunnerCatalogError> {
        self.result
    }
}

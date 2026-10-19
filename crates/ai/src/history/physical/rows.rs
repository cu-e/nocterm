//! The sole actual ordered transcript slab contains genuine row lifetime owners.
//! Replacing the slab aliases unchanged row payloads; it never clones old text.
use super::{
    bundle::{Body, BundleOwner, BundleSlot},
    content::ContentSlot,
    slots::*,
    tools::ToolSlot,
};
mod user;
use crate::{
    model::{
        json::lifetime::Shared,
        native::NativeOwned,
        retention::{
            CensusPermit, RetentionError,
            document::{ConstructionRole, Publication},
            graph::Count,
        },
    },
    thread::Entry,
};
pub use user::UserSlot;

pub struct RowSlot {
    payload: BundleSlot<RowBody>,
    policy: Option<super::policy::ComponentPolicy>,
}
#[expect(
    clippy::large_enum_variant,
    reason = "row bundles charge the inline body size; boxing would move bytes outside that charge"
)]
pub enum RowBody {
    User(UserSlot),
    Agent(TextSlot),
    Thought(TextSlot),
    Content(ContentSlot),
    Tool(ToolSlot),
}
impl RowSlot {
    fn certified(payload: BundleSlot<RowBody>) -> Result<Self, PhysicalError> {
        let mut row = Self {
            payload,
            policy: None,
        };
        row.policy = Some(super::policy::ComponentPolicy::row(&row)?);
        Ok(row)
    }
    pub(crate) fn cached_policy(&self) -> Result<usize, crate::history::MeasurementError> {
        self.policy
            .map(super::policy::ComponentPolicy::bytes)
            .ok_or(crate::history::MeasurementError::Retention(
                RetentionError::UncertifiedPolicy,
            ))
    }
    pub(super) fn new(
        value: Entry,
        parent: &mut SlabBuilder<'_>,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        let mut builder = SlabBuilder::new(parent.role())?;
        let body = match value {
            Entry::Agent(text) => RowBody::Agent(TextSlot::new(text, pool)?),
            Entry::Thought(text) => RowBody::Thought(TextSlot::new(text, pool)?),
            Entry::User(content) => {
                RowBody::User(UserSlot::new(content, &mut builder, original, pool)?)
            }
            Entry::Content(content) => {
                RowBody::Content(ContentSlot::new(content, parent.role(), original, pool)?)
            }
            Entry::Tool(tool) => RowBody::Tool(ToolSlot::new(tool, parent.role(), original, pool)?),
        };
        Self::certified(BundleSlot::new(BundleOwner::build(
            body, builder, original,
        )?))
    }
    fn from_native(
        role: &ConstructionRole,
        value: NativeOwned<Entry>,
    ) -> Result<Self, PhysicalError> {
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let mut builder = SlabBuilder::new(role)?;
        Self::new(source.take(), &mut builder, &original, source.pool())
    }

    fn alias(&self) -> Self {
        Self {
            payload: self.payload.alias(),
            policy: self.policy,
        }
    }
    fn tool(
        tool: ToolSlot,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
    ) -> Result<Self, PhysicalError> {
        Self::certified(BundleSlot::new(BundleOwner::build(
            RowBody::Tool(tool),
            SlabBuilder::new(role)?,
            original,
        )?))
    }
    pub fn view(&self) -> &RowBody {
        self.payload.body()
    }
}
impl Body for RowBody {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        match self {
            Self::Agent(text) | Self::Thought(text) => text.count(count)?,
            Self::User(values) => values.count(count)?,
            Self::Content(value) => value.count(count)?,
            Self::Tool(value) => value.count(count)?,
        }
        Ok(())
    }
    fn retire(&self) {
        match self {
            Self::Agent(text) | Self::Thought(text) => text.retire(),
            Self::User(values) => values.retire(),
            Self::Content(value) => value.retire(),
            Self::Tool(value) => value.retire(),
        }
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        match self {
            Self::Agent(text) | Self::Thought(text) => text.cleanup(permit),
            Self::User(values) => values.cleanup(permit),
            Self::Content(value) => value.cleanup(permit),
            Self::Tool(value) => value.cleanup(permit),
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        match self {
            Self::Agent(text) | Self::Thought(text) => text.adopt(publication),
            Self::User(values) => values.adopt(publication),
            Self::Content(value) => value.adopt(publication),
            Self::Tool(value) => value.adopt(publication),
        }
    }
}

/// Row indices changed by one transition, allocated once at exact size.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ChangedRows(Vec<usize>);
impl ChangedRows {
    pub fn as_slice(&self) -> &[usize] {
        &self.0
    }
}
impl IntoIterator for ChangedRows {
    type Item = usize;
    type IntoIter = std::vec::IntoIter<usize>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

pub struct TranscriptSlot {
    rows: Vec<RowSlot>,
    times: super::source_pool::RetainedBacking<Vec<u64>>,
}
impl TranscriptSlot {
    pub(super) fn policy_capacities(&self) -> (usize, usize) {
        (self.rows.capacity(), self.times.get().capacity())
    }
    pub(super) fn new(
        rows: Vec<RowSlot>,
        times: Vec<u64>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        let bytes = vector_bytes(&times)?;
        Ok(Self {
            rows,
            times: pool.retained(times, bytes)?,
        })
    }
    pub fn rows(&self) -> &[RowSlot] {
        &self.rows
    }
    pub fn times(&self) -> &[u64] {
        &self.times
    }
    pub(super) fn prepare_append_prompt(
        &self,
        role: &ConstructionRole,
        prompt: &super::PromptSlot,
        now: u64,
        original: &Shared<NativeCredit>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut builder = SlabBuilder::new(role)?;
        let user = UserSlot::from_prompt(prompt, &mut builder)?;
        let row = RowSlot::certified(BundleSlot::new(BundleOwner::build(
            RowBody::User(user),
            builder,
            prompt.original_credit(),
        )?))?;
        self.appended(role, row, now, original)
    }
    pub(super) fn prepare_append(
        &self,
        role: &ConstructionRole,
        row: NativeOwned<Entry>,
        now: u64,
        original: &Shared<NativeCredit>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let row = RowSlot::from_native(role, row)?;
        self.appended(role, row, now, original)
    }
    fn appended(
        &self,
        role: &ConstructionRole,
        row: RowSlot,
        now: u64,
        original: &Shared<NativeCredit>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let next_len = self
            .rows
            .len()
            .checked_add(1)
            .ok_or(RetentionError::Arithmetic)?;
        let mut builder = SlabBuilder::new(role)?;
        let mut rows = builder.vector(next_len)?;
        for old in &self.rows {
            rows.push(old.alias());
        }
        rows.push(row);
        let mut times = builder.vector(next_len)?;
        times.extend(self.times.iter().take(self.rows.len()).copied());
        times.resize(self.rows.len(), 0);
        times.push(now);
        Ok(BundleSlot::new(BundleOwner::build(
            Self {
                rows,
                times: super::source_pool::RetainedBacking::from_slab(times),
            },
            builder,
            original,
        )?))
    }
    pub(super) fn prepare_replace(
        &self,
        role: &ConstructionRole,
        index: usize,
        row: NativeOwned<Entry>,
        original: &Shared<NativeCredit>,
    ) -> Result<Option<BundleSlot<Self>>, PhysicalError> {
        if index >= self.rows.len() {
            return Ok(None);
        }
        let replacement = RowSlot::from_native(role, row)?;
        Ok(Some(self.rebuilt(role, index, replacement, original)?))
    }
    /// Patches the first tool row with the patch id. Unchanged rows and every
    /// unchanged tool payload bundle alias the previous transcript version.
    pub(super) fn prepare_patch_tool(
        &self,
        role: &ConstructionRole,
        patch: NativeOwned<crate::model::tools::ToolPatch>,
        original: &Shared<NativeCredit>,
    ) -> Result<Option<BundleSlot<Self>>, PhysicalError> {
        if !patch.ledger().belongs_to_document(role.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let found = self
            .rows
            .iter()
            .enumerate()
            .find_map(|(index, row)| match row.view() {
                RowBody::Tool(tool) if *tool.id() == patch.get().id => Some((index, tool)),
                _ => None,
            });
        let Some((index, tool)) = found else {
            return Ok(None);
        };
        if !tool.changed_by(patch.get()) {
            return Ok(None);
        }
        let mut source = NativeSource::new(patch, role)?;
        let credit = source.original().clone();
        let Some(tool) = tool.patched(source.take(), role, &credit, source.pool())? else {
            return Ok(None);
        };
        let replacement = RowSlot::certified(BundleSlot::new(BundleOwner::build(
            RowBody::Tool(tool),
            SlabBuilder::new(role)?,
            &credit,
        )?))?;
        Ok(Some(self.rebuilt(role, index, replacement, original)?))
    }
    /// Appends one user content item. A final User row is versioned with all
    /// earlier items aliased; otherwise a new User row is appended at `now`.
    pub(super) fn prepare_append_user(
        &self,
        role: &ConstructionRole,
        content: NativeOwned<crate::model::content::Content>,
        now: u64,
        original: &Shared<NativeCredit>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        if !content.ledger().belongs_to_document(role.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let mut source = NativeSource::new(content, role)?;
        let credit = source.original().clone();
        let item = user::item(role, source.take(), &credit, source.pool())?;
        let mut builder = SlabBuilder::new(role)?;
        if let Some(RowBody::User(last)) = self.rows.last().map(RowSlot::view) {
            let body = RowBody::User(last.appended(item, &mut builder)?);
            let replacement =
                RowSlot::certified(BundleSlot::new(BundleOwner::build(body, builder, &credit)?))?;
            return self.rebuilt(role, self.rows.len() - 1, replacement, original);
        }
        let body = RowBody::User(UserSlot::single(item, &mut builder)?);
        let row = RowSlot::certified(BundleSlot::new(BundleOwner::build(body, builder, &credit)?))?;
        self.appended(role, row, now, original)
    }
    /// Appends an agent/thought text chunk. A final row of the same kind is
    /// replaced by one prepaid exact concatenation; every other row aliases.
    /// An empty chunk merged into a matching row is not a mutation (`None`).
    pub(super) fn prepare_append_text(
        &self,
        role: &ConstructionRole,
        thought: bool,
        chunk: NativeOwned<String>,
        now: u64,
        original: &Shared<NativeCredit>,
    ) -> Result<Option<BundleSlot<Self>>, PhysicalError> {
        if !chunk.ledger().belongs_to_document(role.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let last = self
            .rows
            .last()
            .and_then(|row| match (row.view(), thought) {
                (RowBody::Agent(text), false) | (RowBody::Thought(text), true) => Some(text),
                _ => None,
            });
        let Some(previous) = last else {
            let entry = chunk.into_parts();
            let entry = NativeOwned::from_parts(
                if thought {
                    Entry::Thought(entry.0)
                } else {
                    Entry::Agent(entry.0)
                },
                entry.1,
            );
            return self.prepare_append(role, entry, now, original).map(Some);
        };
        if chunk.as_str().is_empty() {
            return Ok(None);
        }
        let mut builder = SlabBuilder::new(role)?;
        let text = {
            let previous = previous.read();
            let capacity = previous
                .len()
                .checked_add(chunk.as_str().len())
                .ok_or(RetentionError::Arithmetic)?;
            let mut text = builder.text(capacity)?;
            text.push_str(&previous);
            text.push_str(chunk.as_str());
            text
        };
        drop(chunk);
        let text = TextSlot::from_slab(text);
        let body = if thought {
            RowBody::Thought(text)
        } else {
            RowBody::Agent(text)
        };
        let replacement = // The builder alone pays for the copy; the consumed chunk's
            // reservation ends here, and no earlier row incarnation is chained.
            RowSlot::certified(BundleSlot::new(BundleOwner::build(body, builder, original)?))?;
        Ok(Some(self.rebuilt(
            role,
            self.rows.len() - 1,
            replacement,
            original,
        )?))
    }
    /// Installs a client display on the tool at `index`; `None` when the row
    /// is not a tool or already shows an equal display.
    pub(super) fn prepare_patch_display(
        &self,
        role: &ConstructionRole,
        index: usize,
        display: NativeOwned<crate::model::tools::ClientDisplay>,
        original: &Shared<NativeCredit>,
    ) -> Result<Option<BundleSlot<Self>>, PhysicalError> {
        if !display.ledger().belongs_to_document(role.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let Some(RowBody::Tool(tool)) = self.rows.get(index).map(RowSlot::view) else {
            return Ok(None);
        };
        let Some(tool) = tool.displayed(display, role, original)? else {
            return Ok(None);
        };
        let replacement = RowSlot::tool(tool, role, original)?;
        Ok(Some(self.rebuilt(role, index, replacement, original)?))
    }
    /// Fails every Pending/Running tool in one pass: one new transcript vector,
    /// all unchanged rows aliased. `None` when no tool was unfinished.
    pub(super) fn prepare_finish_pending_tools(
        &self,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
    ) -> Result<Option<(BundleSlot<Self>, ChangedRows)>, PhysicalError> {
        let unfinished = |row: &RowSlot| match row.view() {
            RowBody::Tool(tool) => tool.interrupted(),
            _ => None,
        };
        let changed_len = self
            .rows
            .iter()
            .filter(|row| {
                matches!(row.view(), RowBody::Tool(tool) if matches!(
                    tool.status(),
                    crate::model::tools::ToolStatus::Pending
                        | crate::model::tools::ToolStatus::Running
                ))
            })
            .count();
        if changed_len == 0 {
            return Ok(None);
        }
        let mut changed = Vec::new();
        changed.try_reserve_exact(changed_len)?;
        let mut builder = SlabBuilder::new(role)?;
        let mut rows = builder.vector(self.rows.len())?;
        for (index, old) in self.rows.iter().enumerate() {
            rows.push(match unfinished(old) {
                Some(tool) => {
                    changed.push(index);
                    RowSlot::tool(tool, role, original)?
                }
                None => old.alias(),
            });
        }
        let mut times = builder.vector(self.times.len())?;
        times.extend(self.times.iter().copied());
        Ok(Some((
            BundleSlot::new(BundleOwner::build(
                Self {
                    rows,
                    times: super::source_pool::RetainedBacking::from_slab(times),
                },
                builder,
                original,
            )?),
            ChangedRows(changed),
        )))
    }
    fn rebuilt(
        &self,
        role: &ConstructionRole,
        index: usize,
        replacement: RowSlot,
        original: &Shared<NativeCredit>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut builder = SlabBuilder::new(role)?;
        let mut rows = builder.vector(self.rows.len())?;
        let mut replacement = Some(replacement);
        for (position, old) in self.rows.iter().enumerate() {
            rows.push(if position == index {
                replacement.take().expect("one concrete row replacement")
            } else {
                old.alias()
            });
        }
        let mut times = builder.vector(self.times.len())?;
        times.extend(self.times.iter().copied());
        Ok(BundleSlot::new(BundleOwner::build(
            Self {
                rows,
                times: super::source_pool::RetainedBacking::from_slab(times),
            },
            builder,
            original,
        )?))
    }
    pub(super) fn prepare_truncate(
        &self,
        role: &ConstructionRole,
        len: usize,
        original: &Shared<NativeCredit>,
    ) -> Result<Option<BundleSlot<Self>>, PhysicalError> {
        let rows_len = len.min(self.rows.len());
        let times_len = len.min(self.times.len());
        if rows_len == self.rows.len() && times_len == self.times.len() {
            return Ok(None);
        }
        let mut builder = SlabBuilder::new(role)?;
        let mut rows = builder.vector(rows_len)?;
        for old in &self.rows[..rows_len] {
            rows.push(old.alias());
        }
        let mut times = builder.vector(times_len)?;
        times.extend(self.times[..times_len].iter().copied());
        Ok(Some(BundleSlot::new(BundleOwner::build(
            Self {
                rows,
                times: super::source_pool::RetainedBacking::from_slab(times),
            },
            builder,
            original,
        )?)))
    }
}
impl Body for TranscriptSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        count.add_exclusive(vector_bytes(&self.rows)?)?;
        self.times.count_metadata(count)?;
        count.add_exclusive(vector_bytes(&self.times)?)?;
        for row in &self.rows {
            row.payload.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.times.cleanup(permit);
        for row in &self.rows {
            row.payload.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.times.adopt(publication);
        for row in &self.rows {
            row.payload.adopt(publication);
        }
    }
    fn retire(&self) {
        for row in &self.rows {
            row.payload.retire();
        }
    }
}

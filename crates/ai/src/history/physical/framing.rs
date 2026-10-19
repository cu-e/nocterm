//! Resume edits alias independent session/path/mode and persisted choice owners.
use super::{
    bundle::{self, Body, BundleOwner, BundleSlot},
    slots::*,
};
use crate::{
    history::ResumeDescriptor,
    model::{
        json::lifetime::Shared,
        native::NativeOwned,
        retention::{
            CensusPermit, RetentionError,
            document::{ConstructionRole, Publication},
            graph::Count,
        },
    },
    session_config::ConfigValue,
};
use std::path::PathBuf;
mod metadata;
#[cfg(test)]
mod tests;
pub use metadata::MetadataSlot;
mod choice;
pub use choice::{ChoiceSlot, ChoiceText, ChoiceValueSlot};
mod path;
use path::ResumePath;
pub struct ResumeSlot {
    session_id: Option<BundleSlot<TextSlot>>,
    workdir: Option<BundleSlot<ResumePath>>,
    config: Vec<ChoiceSlot>,
    mode: Option<BundleSlot<TextSlot>>,
    pending_history: bool,
}
impl ResumeSlot {
    pub(super) fn new(
        value: ResumeDescriptor,
        builder: &mut SlabBuilder<'_>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        let mut config = builder.vector(value.config.len())?;
        let role = builder.role();
        let original = pool.original().clone();
        for (key, value) in value.config {
            config.push(ChoiceSlot::new(key, value, role, &original, pool)?);
        }
        Ok(Self {
            session_id: text(value.session_id, role, &original, pool)?,
            workdir: path(value.workdir, role, &original, pool)?,
            config,
            mode: text(value.mode, role, &original, pool)?,
            pending_history: value.pending_history,
        })
    }
    fn alias(&self, builder: &mut SlabBuilder<'_>) -> Result<Self, PhysicalError> {
        let mut config = builder.vector(self.config.len())?;
        config.extend(self.config.iter().map(ChoiceSlot::alias));
        Ok(Self {
            session_id: self.session_id.as_ref().map(BundleSlot::alias),
            workdir: self.workdir.as_ref().map(BundleSlot::alias),
            config,
            mode: self.mode.as_ref().map(BundleSlot::alias),
            pending_history: self.pending_history,
        })
    }
    pub(in crate::history::physical) fn replacing_session(
        &self,
        value: NativeOwned<Option<String>>,
        role: &ConstructionRole,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        self.replacing_text(value, role, true)
    }
    pub(in crate::history::physical) fn replacing_mode(
        &self,
        value: NativeOwned<Option<String>>,
        role: &ConstructionRole,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        self.replacing_text(value, role, false)
    }
    fn replacing_text(
        &self,
        value: NativeOwned<Option<String>>,
        role: &ConstructionRole,
        session: bool,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let mut builder = SlabBuilder::new(role)?;
        let mut body = self.alias(&mut builder)?;
        let value = text(source.take(), role, &original, source.pool())?;
        if session {
            body.session_id = value;
        } else {
            body.mode = value;
        }
        Ok(BundleSlot::new(BundleOwner::build(
            body, builder, &original,
        )?))
    }
    pub(in crate::history::physical) fn replacing_workdir(
        &self,
        value: NativeOwned<Option<PathBuf>>,
        role: &ConstructionRole,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let mut builder = SlabBuilder::new(role)?;
        let mut body = self.alias(&mut builder)?;
        body.workdir = path(source.take(), role, &original, source.pool())?;
        Ok(BundleSlot::new(BundleOwner::build(
            body, builder, &original,
        )?))
    }
    pub(in crate::history::physical) fn with_pending_history(
        &self,
        pending: bool,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut builder = SlabBuilder::new(role)?;
        let mut body = self.alias(&mut builder)?;
        body.pending_history = pending;
        Ok(BundleSlot::new(BundleOwner::build(
            body, builder, original,
        )?))
    }
    #[cfg(test)]
    pub(in crate::history::physical) fn clear_session(
        &self,
        pending: bool,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut builder = SlabBuilder::new(role)?;
        let mut body = self.alias(&mut builder)?;
        body.session_id = None;
        body.pending_history = pending;
        Ok(BundleSlot::new(BundleOwner::build(
            body, builder, original,
        )?))
    }
    pub(in crate::history::physical) fn replacing_choice(
        &self,
        key: NativeOwned<String>,
        value: NativeOwned<ConfigValue>,
        role: &ConstructionRole,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut source = NativeSource::new(key.join(value)?, role)?;
        let original = source.original().clone();
        let (key, value) = source.take();
        let position = self
            .config
            .binary_search_by(|choice| (*choice.key()).cmp(key.as_str()));
        let mut builder = SlabBuilder::new(role)?;
        let mut config = builder.vector(self.config.len() + usize::from(position.is_err()))?;
        match position {
            Ok(index) => {
                for choice in &self.config[..index] {
                    config.push(choice.alias());
                }
                config.push(self.config[index].replacing_value(
                    value,
                    role,
                    &original,
                    source.pool(),
                )?);
                for choice in &self.config[index + 1..] {
                    config.push(choice.alias());
                }
            }
            Err(index) => {
                for choice in &self.config[..index] {
                    config.push(choice.alias());
                }
                config.push(ChoiceSlot::new(key, value, role, &original, source.pool())?);
                for choice in &self.config[index..] {
                    config.push(choice.alias());
                }
            }
        }
        let body = Self {
            session_id: self.session_id.as_ref().map(BundleSlot::alias),
            workdir: self.workdir.as_ref().map(BundleSlot::alias),
            config,
            mode: self.mode.as_ref().map(BundleSlot::alias),
            pending_history: self.pending_history,
        };
        Ok(BundleSlot::new(BundleOwner::build(
            body, builder, &original,
        )?))
    }
    pub fn session_id(&self) -> Option<TextGuard<'_>> {
        self.session_id.as_ref().map(|value| value.body().read())
    }
    pub fn workdir(&self) -> Option<PathGuard<'_>> {
        self.workdir.as_ref().map(|value| value.body().read())
    }
    pub fn config(&self) -> &[ChoiceSlot] {
        &self.config
    }
    pub fn mode(&self) -> Option<TextGuard<'_>> {
        self.mode.as_ref().map(|value| value.body().read())
    }
    pub fn pending_history(&self) -> bool {
        self.pending_history
    }
}
fn text(
    value: Option<String>,
    role: &ConstructionRole,
    original: &Shared<NativeCredit>,
    pool: &mut NativeSourcePool,
) -> Result<Option<BundleSlot<TextSlot>>, PhysicalError> {
    value
        .map(|value| bundle::part(role, original, TextSlot::new(value, pool)?))
        .transpose()
}
fn path(
    value: Option<PathBuf>,
    role: &ConstructionRole,
    original: &Shared<NativeCredit>,
    pool: &mut NativeSourcePool,
) -> Result<Option<BundleSlot<ResumePath>>, PhysicalError> {
    value
        .map(|value| {
            let path = ResumePath::new(PathSlot::new(value, pool)?);
            bundle::part(role, original, path)
        })
        .transpose()
}

impl Body for ResumeSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        for value in [&self.session_id, &self.mode].into_iter().flatten() {
            value.count(count)?;
        }
        if let Some(value) = &self.workdir {
            value.count(count)?;
        }
        count.add_exclusive(vector_bytes(&self.config)?)?;
        for value in &self.config {
            value.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        for value in [&self.session_id, &self.mode].into_iter().flatten() {
            value.cleanup(permit);
        }
        if let Some(value) = &self.workdir {
            value.cleanup(permit);
        }
        for value in &self.config {
            value.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        for value in [&self.session_id, &self.mode].into_iter().flatten() {
            value.adopt(publication);
        }
        if let Some(value) = &self.workdir {
            value.adopt(publication);
        }
        for value in &self.config {
            value.adopt(publication);
        }
    }
    fn retire(&self) {
        for value in [&self.session_id, &self.mode].into_iter().flatten() {
            value.retire();
        }
        if let Some(value) = &self.workdir {
            value.retire();
        }
        for value in &self.config {
            value.retire();
        }
    }
}

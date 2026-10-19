//! Real transient plan storage follows Live graph lifetime, independently of inputs.
mod controls;
mod usage;
use super::{
    bundle::{self, Body, BundleSlot},
    slots::{NativeCredit, NativeSource, PayloadGuard, PhysicalError, vector_bytes},
    source_pool::NativeBacking,
};
use crate::model::{
    native::NativeOwned,
    notice::PlanStep,
    retention::{
        CensusPermit, RetentionError,
        document::{ConstructionRole, Publication},
        graph::Count,
    },
};
pub use controls::*;
use std::sync::atomic::{AtomicUsize, Ordering};
pub use usage::{LiveUsage, RateLimitSlot};

pub struct LivePresentation {
    plan: Option<BundleSlot<PlanStorage>>,
    usage: Option<BundleSlot<LiveUsage>>,
    controls: Option<BundleSlot<LiveControls>>,
}
struct PlanStorage {
    value: NativeBacking<Vec<PlanStep>>,
    bytes: AtomicUsize,
}
/// Closed updates of real transient components, never an allocation callback.
pub enum PresentationUpdate {
    Info(NativeOwned<crate::connection::AgentInfo>),
    Opened(NativeOwned<crate::model::session::SessionOpened>),
    Plan(NativeOwned<Vec<PlanStep>>),
    Usage(NativeOwned<crate::model::notice::UsageNotice>),
    Commands(NativeOwned<Vec<crate::model::notice::AdvertisedCommand>>),
    Configuration(NativeOwned<Vec<crate::model::controls::ConfigOption>>),
    Mode(NativeOwned<crate::model::ids::ModeId>),
    Tokens(Option<crate::model::usage::TokenTotals>),
}
impl LivePresentation {
    pub fn controls(&self) -> Option<&LiveControls> {
        self.controls.as_ref().map(BundleSlot::body)
    }
    pub(super) fn updated(
        role: &ConstructionRole,
        previous: Option<&Self>,
        update: PresentationUpdate,
        original: &crate::model::json::lifetime::Shared<NativeCredit>,
    ) -> Result<Option<BundleSlot<Self>>, PhysicalError> {
        let controls = match update {
            PresentationUpdate::Opened(value) => {
                LiveControls::replace_opened(role, previous.and_then(Self::controls), value)?
            }
            PresentationUpdate::Info(value) => {
                LiveControls::replace_info(role, previous.and_then(Self::controls), value)?
            }
            PresentationUpdate::Plan(value) => {
                if previous
                    .and_then(Self::plan)
                    .is_some_and(|plan| *plan == *value.get())
                {
                    return Ok(None);
                }
                return Self::replace_plan(previous, role, value).map(Some);
            }
            PresentationUpdate::Usage(value) => {
                return Self::replace_usage(role, previous, value).map(Some);
            }
            PresentationUpdate::Tokens(tokens) => {
                let usage =
                    LiveUsage::finished(role, previous.and_then(Self::usage), tokens, original)?;
                return bundle::part(
                    role,
                    original,
                    Self {
                        plan: previous
                            .and_then(|old| old.plan.as_ref())
                            .map(BundleSlot::alias),
                        usage: Some(usage),
                        controls: previous
                            .and_then(|old| old.controls.as_ref())
                            .map(BundleSlot::alias),
                    },
                )
                .map(Some);
            }
            PresentationUpdate::Commands(value) => {
                LiveControls::replace_commands(role, previous.and_then(Self::controls), value)?
            }
            PresentationUpdate::Configuration(value) => {
                LiveControls::replace_configuration(role, previous.and_then(Self::controls), value)?
            }
            PresentationUpdate::Mode(value) => {
                LiveControls::replace_mode(role, previous.and_then(Self::controls), value)?
            }
        };
        let credit = controls.original_credit().clone();
        bundle::part(
            role,
            &credit,
            Self {
                plan: previous
                    .and_then(|old| old.plan.as_ref())
                    .map(BundleSlot::alias),
                usage: previous
                    .and_then(|old| old.usage.as_ref())
                    .map(BundleSlot::alias),
                controls: Some(controls),
            },
        )
        .map(Some)
    }
    pub fn plan(&self) -> Option<PayloadGuard<'_, Vec<PlanStep>>> {
        self.plan
            .as_ref()
            .map(|slot| PayloadGuard(slot.body().value.read()))
    }
    pub fn usage(&self) -> Option<&LiveUsage> {
        self.usage.as_ref().map(BundleSlot::body)
    }
    pub(super) fn replace_usage(
        role: &ConstructionRole,
        previous: Option<&Self>,
        value: NativeOwned<crate::model::notice::UsageNotice>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let usage = LiveUsage::prepare(role, previous.and_then(Self::usage), value)?;
        let original = usage.original_credit().clone();
        bundle::part(
            role,
            &original,
            Self {
                plan: previous
                    .and_then(|old| old.plan.as_ref())
                    .map(BundleSlot::alias),
                usage: Some(usage),
                controls: previous
                    .and_then(|old| old.controls.as_ref())
                    .map(BundleSlot::alias),
            },
        )
    }
    pub(super) fn replace_plan(
        previous: Option<&Self>,
        role: &ConstructionRole,
        value: NativeOwned<Vec<PlanStep>>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let value = source.take();
        let bytes = value.iter().try_fold(vector_bytes(&value)?, |sum, step| {
            sum.checked_add(step.content.capacity())
                .ok_or(RetentionError::Arithmetic)
        })?;
        let plan = bundle::part(
            role,
            &original,
            PlanStorage {
                value: NativeBacking::from_source(value, bytes, source.pool())?,
                bytes: AtomicUsize::new(bytes),
            },
        )?;
        bundle::part(
            role,
            &original,
            Self {
                plan: Some(plan),
                usage: previous
                    .and_then(|old| old.usage.as_ref())
                    .map(BundleSlot::alias),
                controls: previous
                    .and_then(|old| old.controls.as_ref())
                    .map(BundleSlot::alias),
            },
        )
    }
}
impl Body for PlanStorage {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.value.count_metadata(count)?;
        count.add_exclusive(self.bytes.load(Ordering::Acquire))
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.value.cleanup(permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.value.adopt(publication);
    }
    fn retire(&self) {
        self.value.retire();
        self.bytes.store(0, Ordering::Release);
    }
}
impl Body for LivePresentation {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        if let Some(value) = &self.controls {
            value.count(count)?;
        }
        if let Some(value) = &self.usage {
            value.count(count)?;
        }
        self.plan
            .as_ref()
            .map_or(Ok(()), |value| value.count(count))
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        if let Some(value) = &self.controls {
            value.cleanup(permit);
        }
        if let Some(value) = &self.usage {
            value.cleanup(permit);
        }
        if let Some(value) = &self.plan {
            value.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        if let Some(value) = &self.controls {
            value.adopt(publication);
        }
        if let Some(value) = &self.usage {
            value.adopt(publication);
        }
        if let Some(value) = &self.plan {
            value.adopt(publication);
        }
    }
    fn retire(&self) {
        if let Some(value) = &self.controls {
            value.retire();
        }
        if let Some(value) = &self.usage {
            value.retire();
        }
        if let Some(value) = &self.plan {
            value.retire();
        }
    }
}

//! Usage updates share unchanged physical windows and retain original context payloads.
use super::super::{
    bundle::{self, Body, BundleOwner, BundleSlot},
    slots::{NativeCredit, NativeSource, PayloadGuard, PhysicalError, SlabBuilder, vector_bytes},
    source_pool::{NativeBacking, SourceVector},
};
use crate::{
    model::{
        json::lifetime::Shared,
        native::NativeOwned,
        notice::UsageNotice,
        retention::{
            CensusPermit, RetentionError,
            document::{ConstructionRole, Publication},
            graph::Count,
        },
        usage::{ContextUsage, TokenTotals},
    },
    usage::Window,
};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct LiveUsage {
    context: Option<BundleSlot<ContextSlot>>,
    limits: Vec<BundleSlot<RateLimitSlot>>,
    tokens: Option<TokenTotals>,
}
struct ContextSlot {
    value: NativeBacking<ContextUsage>,
    bytes: AtomicUsize,
}
struct WindowSlot {
    value: NativeBacking<Window>,
    bytes: AtomicUsize,
}
pub struct RateLimitSlot {
    window: BundleSlot<WindowSlot>,
    used: Option<f64>,
    resets_at: Option<u64>,
    reached: bool,
}
impl RateLimitSlot {
    pub fn window(&self) -> PayloadGuard<'_, Window> {
        PayloadGuard(self.window.body().value.read())
    }
    pub fn used(&self) -> Option<f64> {
        self.used
    }
    pub fn resets_at(&self) -> Option<u64> {
        self.resets_at
    }
    pub fn reached(&self) -> bool {
        self.reached
    }
}
impl LiveUsage {
    pub fn context(&self) -> Option<PayloadGuard<'_, ContextUsage>> {
        self.context
            .as_ref()
            .map(|slot| PayloadGuard(slot.body().value.read()))
    }
    pub fn limits(&self) -> impl ExactSizeIterator<Item = &RateLimitSlot> {
        self.limits.iter().map(BundleSlot::body)
    }
    pub fn tokens(&self) -> Option<&TokenTotals> {
        self.tokens.as_ref()
    }
    pub(in crate::history::physical) fn prepare(
        role: &ConstructionRole,
        previous: Option<&Self>,
        value: NativeOwned<UsageNotice>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let value = source.take();
        let mut builder = SlabBuilder::new(role)?;
        let capacity = previous
            .map_or(0, |old| old.limits.len())
            .checked_add(value.rate_limits.len())
            .ok_or(RetentionError::Arithmetic)?;
        let mut limits = builder.vector(capacity)?;
        if let Some(previous) = previous {
            limits.extend(previous.limits.iter().map(BundleSlot::alias));
        }
        let context = if let Some(old) = previous.and_then(|old| old.context.as_ref())
            && *PayloadGuard(old.body().value.read()) == value.context
        {
            old.alias()
        } else {
            let bytes = value
                .context
                .cost
                .as_ref()
                .map_or(0, |cost| cost.currency.capacity());
            bundle::part(
                role,
                &original,
                ContextSlot {
                    value: NativeBacking::from_source(value.context, bytes, source.pool())?,
                    bytes: AtomicUsize::new(bytes),
                },
            )?
        };
        let mut supplied = SourceVector::new(value.rate_limits, source.pool())?;
        while let Some(limit) = supplied.next() {
            let position = limits
                .iter()
                .position(|old: &BundleSlot<RateLimitSlot>| *old.body().window() == limit.window);
            let window = match position {
                Some(position) => limits[position].body().window.alias(),
                None => {
                    let bytes = match &limit.window {
                        Window::WeeklyModel(model) => model.capacity(),
                        _ => 0,
                    };
                    bundle::part(
                        role,
                        &original,
                        WindowSlot {
                            value: NativeBacking::from_source(limit.window, bytes, source.pool())?,
                            bytes: AtomicUsize::new(bytes),
                        },
                    )?
                }
            };
            let replacement = bundle::part(
                role,
                &original,
                RateLimitSlot {
                    window,
                    used: limit.used,
                    resets_at: limit.resets_at,
                    reached: limit.reached,
                },
            )?;
            match position {
                Some(position) => limits[position] = replacement,
                None => limits.push(replacement),
            }
        }
        drop(supplied);
        limits.sort_unstable_by(|left, right| left.body().window().cmp(&right.body().window()));
        Ok(BundleSlot::new(BundleOwner::build(
            Self {
                context: Some(context),
                limits,
                tokens: previous.and_then(|old| old.tokens.clone()),
            },
            builder,
            &original,
        )?))
    }
    pub(in crate::history::physical) fn finished(
        role: &ConstructionRole,
        previous: Option<&Self>,
        tokens: Option<TokenTotals>,
        original: &Shared<NativeCredit>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut builder = SlabBuilder::new(role)?;
        let mut limits = builder.vector(previous.map_or(0, |old| old.limits.len()))?;
        if let Some(previous) = previous {
            limits.extend(previous.limits.iter().map(BundleSlot::alias));
        }
        Ok(BundleSlot::new(BundleOwner::build(
            Self {
                context: previous.and_then(|old| old.context.as_ref().map(BundleSlot::alias)),
                limits,
                tokens,
            },
            builder,
            original,
        )?))
    }
}
impl Body for ContextSlot {
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
impl Body for WindowSlot {
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
impl Body for RateLimitSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.window.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.window.cleanup(permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.window.adopt(publication);
    }
    fn retire(&self) {
        self.window.retire();
    }
}
impl Body for LiveUsage {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        count.add_exclusive(vector_bytes(&self.limits)?)?;
        if let Some(context) = &self.context {
            context.count(count)?;
        }
        self.limits.iter().try_for_each(|limit| limit.count(count))
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        if let Some(context) = &self.context {
            context.cleanup(permit);
        }
        self.limits.iter().for_each(|limit| limit.cleanup(permit));
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        if let Some(context) = &self.context {
            context.adopt(publication);
        }
        self.limits
            .iter()
            .for_each(|limit| limit.adopt(publication));
    }
    fn retire(&self) {
        if let Some(context) = &self.context {
            context.retire();
        }
        self.limits.iter().for_each(BundleSlot::retire);
    }
}

#[cfg(test)]
mod tests;

//! Bridge identity evidence as independently aliasable claim components.
//! Merging follows `BridgeClaims::merge`; unchanged components alias payloads.
use super::super::{
    bundle::{self, Body, BundleSlot},
    slots::{NativeCredit, NativeSourcePool, PhysicalError, TextGuard, TextSlot},
};
use crate::model::{
    json::lifetime::Shared,
    retention::{
        CensusPermit, RetentionError,
        document::{ConstructionRole, Publication},
        graph::Count,
    },
    tools::{BridgeClaims, Claim},
};

pub(in crate::history::physical) struct KnownClaim {
    registration: TextSlot,
    tool: TextSlot,
}
impl Body for KnownClaim {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.registration.count(count)?;
        self.tool.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.registration.cleanup(permit);
        self.tool.cleanup(permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.registration.adopt(publication);
        self.tool.adopt(publication);
    }

    fn retire(&self) {
        self.registration.retire();
        self.tool.retire();
    }
}
enum ClaimEntry {
    Absent,
    Invalid,
    Known(BundleSlot<KnownClaim>),
}
/// One claim component; known identities are a shared immutable payload.
pub struct ClaimSlot(ClaimEntry);
impl ClaimSlot {
    fn changed_by(&self, incoming: &Claim) -> bool {
        match (&self.0, incoming) {
            (_, Claim::Absent) | (ClaimEntry::Invalid, _) => false,
            (ClaimEntry::Absent, _) => true,
            (ClaimEntry::Known(_), Claim::Known { registration, tool }) => !self
                .known()
                .is_some_and(|(left, right)| &*left == registration && &*right == tool),
            _ => true,
        }
    }
    fn new(
        value: Claim,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        Ok(Self(match value {
            Claim::Absent => ClaimEntry::Absent,
            Claim::Invalid => ClaimEntry::Invalid,
            Claim::Known { registration, tool } => ClaimEntry::Known(bundle::part(
                role,
                original,
                KnownClaim {
                    registration: TextSlot::new(registration, pool)?,
                    tool: TextSlot::new(tool, pool)?,
                },
            )?),
        }))
    }
    fn alias(&self) -> Self {
        Self(match &self.0 {
            ClaimEntry::Absent => ClaimEntry::Absent,
            ClaimEntry::Invalid => ClaimEntry::Invalid,
            ClaimEntry::Known(value) => ClaimEntry::Known(value.alias()),
        })
    }
    /// `Claim::merge`: equality is borrowed; only a genuinely new claim allocates.
    fn merged(
        &self,
        incoming: Claim,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        match (&self.0, &incoming) {
            (_, Claim::Absent) => Ok(self.alias()),
            (ClaimEntry::Absent, _) => Self::new(incoming, role, original, pool),
            (ClaimEntry::Invalid, Claim::Invalid) => Ok(self.alias()),
            (ClaimEntry::Known(_), Claim::Known { registration, tool })
                if self
                    .known()
                    .is_some_and(|(r, t)| &*r == registration && &*t == tool) =>
            {
                Ok(self.alias())
            }
            _ => Ok(Self(ClaimEntry::Invalid)),
        }
    }
    pub fn is_absent(&self) -> bool {
        matches!(self.0, ClaimEntry::Absent)
    }
    pub fn is_invalid(&self) -> bool {
        matches!(self.0, ClaimEntry::Invalid)
    }
    /// Registration and tool names of a known claim.
    pub fn known(&self) -> Option<(TextGuard<'_>, TextGuard<'_>)> {
        match &self.0 {
            ClaimEntry::Known(value) => {
                let body = value.body();
                Some((body.registration.read(), body.tool.read()))
            }
            _ => None,
        }
    }
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        match &self.0 {
            ClaimEntry::Known(value) => value.count(count),
            _ => Ok(()),
        }
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        if let ClaimEntry::Known(value) = &self.0 {
            value.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        if let ClaimEntry::Known(value) = &self.0 {
            value.adopt(publication);
        }
    }
    fn retire(&self) {
        if let ClaimEntry::Known(value) = &self.0 {
            value.retire();
        }
    }
}
pub struct ClaimsSlot {
    name: ClaimSlot,
    title: ClaimSlot,
    input: ClaimSlot,
    legacy_display_hint: Option<BundleSlot<TextSlot>>,
}
impl ClaimsSlot {
    pub(super) fn changed_by(&self, incoming: &BridgeClaims) -> bool {
        self.name.changed_by(&incoming.name)
            || self.title.changed_by(&incoming.title)
            || self.input.changed_by(&incoming.input)
            || incoming.legacy_display_hint.as_ref().is_some_and(|hint| {
                self.legacy_display_hint
                    .as_ref()
                    .map(|current| current.body().read())
                    .as_deref()
                    != Some(hint.as_str())
            })
    }
    pub(super) fn new(
        value: BridgeClaims,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        Ok(Self {
            name: ClaimSlot::new(value.name, role, original, pool)?,
            title: ClaimSlot::new(value.title, role, original, pool)?,
            input: ClaimSlot::new(value.input, role, original, pool)?,
            legacy_display_hint: value
                .legacy_display_hint
                .map(|hint| bundle::part(role, original, TextSlot::new(hint, pool)?))
                .transpose()?,
        })
    }
    /// `BridgeClaims::merge` over physical components.
    pub(super) fn merged(
        &self,
        incoming: BridgeClaims,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        Ok(Self {
            name: self.name.merged(incoming.name, role, original, pool)?,
            title: self.title.merged(incoming.title, role, original, pool)?,
            input: self.input.merged(incoming.input, role, original, pool)?,
            legacy_display_hint: match incoming.legacy_display_hint {
                Some(hint) => Some(bundle::part(role, original, TextSlot::new(hint, pool)?)?),
                None => self.legacy_display_hint.as_ref().map(BundleSlot::alias),
            },
        })
    }
    pub(super) fn alias(&self) -> Self {
        Self {
            name: self.name.alias(),
            title: self.title.alias(),
            input: self.input.alias(),
            legacy_display_hint: self.legacy_display_hint.as_ref().map(BundleSlot::alias),
        }
    }
    pub(super) fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.name.count(count)?;
        self.title.count(count)?;
        self.input.count(count)?;
        if let Some(hint) = &self.legacy_display_hint {
            hint.count(count)?;
        }
        Ok(())
    }
    pub(super) fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.name.cleanup(permit);
        self.title.cleanup(permit);
        self.input.cleanup(permit);
        if let Some(hint) = &self.legacy_display_hint {
            hint.cleanup(permit);
        }
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        self.name.adopt(publication);
        self.title.adopt(publication);
        self.input.adopt(publication);
        if let Some(hint) = &self.legacy_display_hint {
            hint.adopt(publication);
        }
    }
    pub(super) fn retire(&self) {
        self.name.retire();
        self.title.retire();
        self.input.retire();
        if let Some(hint) = &self.legacy_display_hint {
            hint.retire();
        }
    }
    pub fn name(&self) -> &ClaimSlot {
        &self.name
    }
    pub fn title(&self) -> &ClaimSlot {
        &self.title
    }
    pub fn input(&self) -> &ClaimSlot {
        &self.input
    }
    pub fn legacy_display_hint(&self) -> Option<TextGuard<'_>> {
        self.legacy_display_hint
            .as_ref()
            .map(|hint| hint.body().read())
    }
}

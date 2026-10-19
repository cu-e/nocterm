//! Each persisted choice and its independently shared key/value has one owner.
use super::super::{
    bundle::{self, Body, BundleSlot},
    slots::*,
};
use crate::{
    model::{
        json::lifetime::Shared,
        retention::{
            CensusPermit, RetentionError,
            document::{ConstructionRole, Publication},
            graph::Count,
        },
    },
    session_config::ConfigValue,
};
pub struct ChoiceSlot {
    payload: BundleSlot<ChoiceBody>,
}
struct ChoiceBody {
    key: BundleSlot<TextSlot>,
    value: ChoiceValueSlot,
}
pub enum ChoiceValueSlot {
    Flag(bool),
    Value(ChoiceText),
}
impl ChoiceValueSlot {
    pub fn matches(&self, value: &ConfigValue) -> bool {
        match (self, value) {
            (Self::Flag(left), ConfigValue::Flag(right)) => left == right,
            (Self::Value(left), ConfigValue::Value(right)) => &*left.read() == right.as_str(),
            _ => false,
        }
    }
}
pub struct ChoiceText {
    payload: BundleSlot<TextSlot>,
}
impl ChoiceText {
    pub fn read(&self) -> TextGuard<'_> {
        self.payload.body().read()
    }
}
impl ChoiceSlot {
    pub(in crate::history::physical) fn new(
        key: String,
        value: ConfigValue,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        let key = bundle::part(role, original, TextSlot::new(key, pool)?)?;
        Self::build(key, value, role, original, pool)
    }
    fn build(
        key: BundleSlot<TextSlot>,
        value: ConfigValue,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        let value = match value {
            ConfigValue::Flag(value) => ChoiceValueSlot::Flag(value),
            ConfigValue::Value(value) => ChoiceValueSlot::Value(ChoiceText {
                payload: bundle::part(role, original, TextSlot::new(value, pool)?)?,
            }),
        };
        Ok(Self {
            payload: bundle::part(role, original, ChoiceBody { key, value })?,
        })
    }
    pub(in crate::history::physical) fn replacing_value(
        &self,
        value: ConfigValue,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Self, PhysicalError> {
        Self::build(self.payload.body().key.alias(), value, role, original, pool)
    }
    pub(in crate::history::physical) fn alias(&self) -> Self {
        Self {
            payload: self.payload.alias(),
        }
    }
    pub fn key(&self) -> TextGuard<'_> {
        self.payload.body().key.body().read()
    }
    pub fn value(&self) -> &ChoiceValueSlot {
        &self.payload.body().value
    }
}
impl Body for ChoiceSlot {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.payload.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.payload.cleanup(permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.payload.adopt(publication);
    }
    fn retire(&self) {
        self.payload.retire();
    }
}
impl Body for ChoiceBody {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.key.count(count)?;
        if let ChoiceValueSlot::Value(value) = &self.value {
            value.payload.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.key.cleanup(permit);
        if let ChoiceValueSlot::Value(value) = &self.value {
            value.payload.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.key.adopt(publication);
        if let ChoiceValueSlot::Value(value) = &self.value {
            value.payload.adopt(publication);
        }
    }
    fn retire(&self) {
        self.key.retire();
        if let ChoiceValueSlot::Value(value) = &self.value {
            value.payload.retire();
        }
    }
}

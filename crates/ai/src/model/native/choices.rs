//! Configuration insertion owns its prepaid B-tree envelope and guarded texts.
use super::NativeOwned;
use crate::{
    model::json::deferred::{DecodeLedger, JsonDecodeError, ledger::Reservation},
    session_config::{Choices, ConfigValue, choices_construction_backing_bound},
};

pub struct NativeChoices {
    values: Choices,
    allocation: Reservation,
    remaining: usize,
}
impl NativeChoices {
    pub fn new(ledger: &DecodeLedger, entries: usize) -> Result<Self, JsonDecodeError> {
        let bytes = if entries == 0 {
            0
        } else {
            choices_construction_backing_bound(entries).ok_or(JsonDecodeError::Arithmetic)?
        };
        Ok(Self {
            values: Choices::new(),
            allocation: ledger.reserve(bytes)?,
            remaining: entries,
        })
    }
    pub fn push(
        &mut self,
        key: NativeOwned<String>,
        value: NativeOwned<ConfigValue>,
    ) -> Result<(), JsonDecodeError> {
        if self.remaining == 0 {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        if !key.allocation.belongs_to(self.allocation.original_ledger())
            || !value
                .allocation
                .belongs_to(self.allocation.original_ledger())
        {
            return Err(JsonDecodeError::ForeignLedger);
        }
        let ((key, value), mut allocation) = key.join(value)?.into_parts();
        self.allocation.merge_from(&mut allocation)?;
        self.values.insert(key, value);
        self.remaining -= 1;
        Ok(())
    }
    pub fn finish(self) -> Result<NativeOwned<Choices>, JsonDecodeError> {
        if self.remaining != 0 {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        Ok(NativeOwned::from_parts(self.values, self.allocation))
    }
}
pub fn flag(
    value: bool,
    ledger: &DecodeLedger,
) -> Result<NativeOwned<ConfigValue>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(
        ConfigValue::Flag(value),
        ledger.reserve(0)?,
    ))
}
pub fn text_value(text: NativeOwned<String>) -> NativeOwned<ConfigValue> {
    let (text, allocation) = text.into_parts();
    NativeOwned::from_parts(ConfigValue::Value(text), allocation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{json::deferred::literal::LiteralBuilder, retention::RetentionAuthority};
    fn text(ledger: &DecodeLedger, value: &str) -> NativeOwned<String> {
        let mut builder = LiteralBuilder::with_cleanup_frontier(ledger, 1).unwrap();
        let mut text = builder.text().unwrap();
        text.append(value).unwrap();
        builder.string(text).unwrap();
        NativeOwned::text(builder.finish_native().unwrap().into_text().unwrap())
    }
    #[test]
    fn guarded_choices_keep_string_and_boolean_values_with_last_duplicate_winning() {
        let authority = RetentionAuthority::new(1024 * 1024).unwrap();
        let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
        let baseline = ledger.live_requested_bytes();
        let mut choices = NativeChoices::new(&ledger, 3).unwrap();
        choices
            .push(
                text(&ledger, "flag"),
                crate::model::native::choices::flag(false, &ledger).unwrap(),
            )
            .unwrap();
        choices
            .push(
                text(&ledger, "model"),
                crate::model::native::choices::text_value(text(&ledger, "original")),
            )
            .unwrap();
        choices
            .push(
                text(&ledger, "flag"),
                crate::model::native::choices::flag(true, &ledger).unwrap(),
            )
            .unwrap();
        let (values, allocation) = choices.finish().unwrap().into_parts();
        assert_eq!(values.get("flag"), Some(&ConfigValue::Flag(true)));
        assert_eq!(
            values.get("model"),
            Some(&ConfigValue::Value("original".into()))
        );
        assert_eq!(values.len(), 2);
        drop(values);
        drop(allocation);
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }
    #[test]
    fn foreign_choice_refusal_leaves_existing_values_and_text_reservations_intact() {
        let authority = RetentionAuthority::new(1024 * 1024).unwrap();
        let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
        let foreign = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
        let baseline = foreign.live_requested_bytes();
        let mut choices = NativeChoices::new(&ledger, 1).unwrap();
        assert!(matches!(
            choices.push(
                text(&foreign, "foreign"),
                crate::model::native::choices::flag(true, &foreign).unwrap()
            ),
            Err(JsonDecodeError::ForeignLedger)
        ));
        assert_eq!(foreign.live_requested_bytes(), baseline);
        choices
            .push(
                text(&ledger, "original"),
                crate::model::native::choices::flag(true, &ledger).unwrap(),
            )
            .unwrap();
        let (values, allocation) = choices.finish().unwrap().into_parts();
        assert_eq!(values.get("original"), Some(&ConfigValue::Flag(true)));
        drop(values);
        drop(allocation);
    }
}

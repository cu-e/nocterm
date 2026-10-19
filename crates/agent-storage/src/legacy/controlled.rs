//! Controlled native import exposes only literal evidence and original ordering.
use nocterm_ai::{
    history::HistoryCodecError,
    model::{
        json::deferred::{DecodeLedger, JsonDecodeError},
        literal::LiteralEvidence,
        tools::ToolRecord,
    },
};
pub(crate) type ControlledRowOrder = super::order::RowOrder;
pub(crate) type ControlledFieldOrder = super::order::FieldOrder;
pub(crate) fn controlled_literal(
    name: Option<&str>,
    title: Option<&str>,
    input: Option<&serde_json::Value>,
) -> LiteralEvidence {
    super::identity::literal(name, title, input)
}
pub(crate) fn controlled_order(
    row: &ToolRecord,
    slot: usize,
    fields: ControlledFieldOrder,
) -> Option<ControlledRowOrder> {
    super::order::capture(row, slot, fields)
}
pub(crate) fn controlled_field_order<'a>(
    names: impl Iterator<Item = &'a str>,
) -> ControlledFieldOrder {
    super::order::FieldOrder::from_names(names)
}
pub(crate) struct ControlledGrammar {
    grammar: super::borrowed::Grammar,
}
impl ControlledGrammar {
    pub(crate) fn new(ledger: &DecodeLedger, rows: usize) -> Result<Self, JsonDecodeError> {
        Ok(Self {
            grammar: super::borrowed::Grammar::admitted(ledger, rows)?,
        })
    }
    pub(crate) fn record(
        &mut self,
        row: Option<ControlledRowOrder>,
    ) -> Result<(), HistoryCodecError> {
        self.grammar
            .record_admitted(row)
            .map_err(|error| HistoryCodecError::Validation(nocterm_core::SharedCause::new(error)))
    }
    pub(crate) fn finish(
        self,
        ledger: &DecodeLedger,
    ) -> Result<nocterm_ai::history::retention::LegacyFormat, JsonDecodeError> {
        self.grammar.retained_admitted(ledger)
    }
}

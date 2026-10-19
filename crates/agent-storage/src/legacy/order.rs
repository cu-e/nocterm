//! Private immutable original row, extension and display provenance.
use nocterm_ai::model::tools::ToolRecord;
#[cfg(any(test, feature = "test-support"))]
use serde_json::{Map, Value};
#[derive(Clone, Copy)]
pub(super) enum DisplayField {
    Version,
    Tool,
    Arguments,
    Destination,
    Redacted,
    Outcome,
}
#[derive(Clone, Copy)]
pub(crate) struct FieldOrder {
    fields: [DisplayField; 6],
    len: u8,
}
impl FieldOrder {
    pub(super) const NATIVE: Self = Self {
        fields: [
            DisplayField::Version,
            DisplayField::Tool,
            DisplayField::Arguments,
            DisplayField::Destination,
            DisplayField::Redacted,
            DisplayField::Outcome,
        ],
        len: 6,
    };
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn capture(object: &Map<String, Value>) -> Self {
        let mut order = Self::NATIVE;
        order.len = 0;
        for key in object.keys() {
            let field = match key.as_str() {
                "version" => DisplayField::Version,
                "tool" => DisplayField::Tool,
                "arguments" => DisplayField::Arguments,
                "destination" => DisplayField::Destination,
                "redacted" => DisplayField::Redacted,
                "outcome" => DisplayField::Outcome,
                _ => continue,
            };
            if usize::from(order.len) < order.fields.len() {
                order.fields[usize::from(order.len)] = field;
                order.len += 1;
            }
        }
        order
    }
    pub(super) fn from_names<'a>(names: impl Iterator<Item = &'a str>) -> Self {
        let mut result = Self::NATIVE;
        result.len = 0;
        for name in names {
            let field = match name {
                "version" => DisplayField::Version,
                "tool" => DisplayField::Tool,
                "arguments" => DisplayField::Arguments,
                "destination" => DisplayField::Destination,
                "redacted" => DisplayField::Redacted,
                "outcome" => DisplayField::Outcome,
                _ => continue,
            };
            if usize::from(result.len) < result.fields.len() {
                result.fields[usize::from(result.len)] = field;
                result.len += 1;
            }
        }
        result
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = DisplayField> + '_ {
        self.fields[..usize::from(self.len)].iter().copied()
    }
}
#[derive(Clone, Copy)]
pub(crate) struct OrderFields {
    meta_slot: usize,
    display_fields: FieldOrder,
}
pub(crate) type RowOrder = nocterm_ai::history::retention::LegacyRow<OrderFields>;
pub(super) fn capture(
    row: &ToolRecord,
    meta_slot: usize,
    display_fields: FieldOrder,
) -> Option<RowOrder> {
    Some(RowOrder::new(
        nocterm_ai::history::retention::LegacyRowIdentity::capture(row)?,
        OrderFields {
            meta_slot,
            display_fields,
        },
    ))
}
pub(super) trait RowOrderExt {
    fn managed_slot(&self, row: &nocterm_ai::history::physical::ToolSlot) -> Option<usize>;
    fn managed_fields(&self, row: &nocterm_ai::history::physical::ToolSlot) -> FieldOrder;
    fn matches(&self, row: &ToolRecord) -> bool;
    fn slot(&self, row: &ToolRecord) -> Option<usize>;
    fn fields(&self, row: &ToolRecord) -> FieldOrder;
    fn visit_json_allocations<'a>(
        &'a self,
        visit: &mut dyn FnMut(
            nocterm_ai::model::json::RetainedAllocation<'a>,
        ) -> Result<(), nocterm_ai::history::RetentionError>,
    ) -> Result<(), nocterm_ai::history::RetentionError>;
    fn retained_heap_bytes(&self) -> Option<usize>;
}
impl RowOrderExt for RowOrder {
    fn managed_slot(&self, row: &nocterm_ai::history::physical::ToolSlot) -> Option<usize> {
        self.identity()
            .matches_managed_extensions(row)
            .then_some(self.facts().meta_slot)
    }
    fn managed_fields(&self, row: &nocterm_ai::history::physical::ToolSlot) -> FieldOrder {
        if self.identity().matches_managed_display(row) {
            self.facts().display_fields
        } else {
            FieldOrder::NATIVE
        }
    }

    fn matches(&self, row: &ToolRecord) -> bool {
        self.identity().matches(row)
    }
    fn slot(&self, row: &ToolRecord) -> Option<usize> {
        self.identity()
            .matches_extensions(row)
            .then_some(self.facts().meta_slot)
    }
    fn fields(&self, row: &ToolRecord) -> FieldOrder {
        if self.identity().matches_display(row) {
            self.facts().display_fields
        } else {
            FieldOrder::NATIVE
        }
    }
    fn visit_json_allocations<'a>(
        &'a self,
        visit: &mut dyn FnMut(
            nocterm_ai::model::json::RetainedAllocation<'a>,
        ) -> Result<(), nocterm_ai::history::RetentionError>,
    ) -> Result<(), nocterm_ai::history::RetentionError> {
        self.identity().visit_json_allocations(visit)
    }
    fn retained_heap_bytes(&self) -> Option<usize> {
        self.identity().retained_heap_bytes()
    }
}

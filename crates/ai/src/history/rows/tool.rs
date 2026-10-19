//! Final tool-row metadata and literal argument/result ownership, without JSON bridge.
use super::{DecodeLedger, DecodedRow, JsonDecodeError, size_of};
use crate::{
    model::{
        json::deferred::{DecodedJson, literal::LiteralText},
        tools::{ToolKind, ToolStatus},
    },
    thread::Entry,
};
/// Controlled textual header; final constructor checks every supplied ledger.
pub struct ToolRowHeader {
    id: LiteralText,
    title: LiteralText,
    name: Option<LiteralText>,
    kind: ToolKind,
    status: ToolStatus,
}
impl ToolRowHeader {
    pub fn new(id: LiteralText, title: LiteralText, kind: ToolKind, status: ToolStatus) -> Self {
        Self {
            id,
            title,
            name: None,
            kind,
            status,
        }
    }
    pub fn with_name(mut self, name: LiteralText) -> Self {
        self.name = Some(name);
        self
    }
}
impl DecodedRow {
    /// A genuine final ToolRecord for the empty compound-fields case. Callers
    /// must establish those fields really are empty/None, never silently omit
    /// nonempty output/locations/extensions/display from a source document.
    pub fn tool_without_output(
        header: ToolRowHeader,
        arguments: Option<DecodedJson>,
        result: Option<DecodedJson>,
        ledger: &DecodeLedger,
    ) -> Result<Self, JsonDecodeError> {
        Self::tool_with_display(header, arguments, result, None, ledger)
    }
    /// Same empty-output/location/extensions case, with real controlled display.
    pub fn tool_with_display(
        header: ToolRowHeader,
        arguments: Option<DecodedJson>,
        result: Option<DecodedJson>,
        display: Option<super::DecodedDisplay>,
        ledger: &DecodeLedger,
    ) -> Result<Self, JsonDecodeError> {
        let fixed = ledger.reserve(size_of::<Self>())?;
        use crate::model::native::{NativeList, NativeOwned};
        let id = crate::model::native::tools::tool_id(super::owned_text(header.id, ledger)?)?;
        let title = super::owned_text(header.title, ledger)?;
        let name = match header.name {
            Some(name) => NativeOwned::supplied(super::owned_text(name, ledger)?),
            None => NativeOwned::absent(ledger)?,
        };
        let header =
            crate::model::native::tools::header(id, title, name, header.kind, header.status)?;
        let arguments = match arguments {
            Some(value) => NativeOwned::supplied(super::owned_json(value, ledger)?),
            None => NativeOwned::absent(ledger)?,
        };
        let result = match result {
            Some(value) => NativeOwned::supplied(super::owned_json(value, ledger)?),
            None => NativeOwned::absent(ledger)?,
        };
        let display = match display {
            Some(display) => {
                let (display, allocation) = display.into_parts(ledger)?;
                NativeOwned::supplied(NativeOwned::from_parts(display, allocation))
            }
            None => NativeOwned::absent(ledger)?,
        };
        let record = crate::model::native::tools::record(
            header,
            NativeList::new(ledger, 0)?.finish(),
            NativeList::new(ledger, 0)?.finish(),
            arguments,
            result,
            NativeOwned::absent(ledger)?,
            display,
        )?;
        let (record, backing) = record.into_parts();
        Ok(Self {
            entry: Entry::Tool(record),
            _backing: backing,
            _fixed: fixed,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::json::deferred::literal::LiteralBuilder;
    fn text(value: &str, ledger: &DecodeLedger) -> LiteralText {
        let mut text = LiteralText::new(ledger).unwrap();
        text.append(value).unwrap();
        text
    }
    #[test]
    fn final_tool_row_moves_deep_literal_json_and_retains_original_immutable_receipt() {
        let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
        let initial = ledger.live_requested_bytes();
        let mut json = LiteralBuilder::new(&ledger).unwrap();
        for _ in 0..200 {
            json.start_array().unwrap();
        }
        json.null().unwrap();
        for _ in 0..200 {
            json.end_container().unwrap();
        }
        let arguments = json.finish().unwrap();
        let pointer = arguments.as_owned().as_value() as *const _;
        let header = ToolRowHeader::new(
            text("original", &ledger),
            text("deep", &ledger),
            ToolKind::Other,
            ToolStatus::Pending,
        );
        let row = DecodedRow::tool_without_output(header, Some(arguments), None, &ledger).unwrap();
        let Entry::Tool(call) = row.as_entry() else {
            panic!("tool")
        };
        assert_eq!(
            call.arguments.as_ref().unwrap().as_value() as *const _,
            pointer
        );
        assert!(call.result.is_none());
        let mut value = call.arguments.as_ref().unwrap().as_value();
        for _ in 0..200 {
            value = &value.as_array().unwrap()[0];
        }
        assert!(value.is_null());
        drop(row);
        assert_eq!(ledger.live_requested_bytes(), initial);
    }
    #[test]
    fn final_tool_row_keeps_supplied_null_distinct_from_absent_result() {
        let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 16384).unwrap();
        let mut json = LiteralBuilder::new(&ledger).unwrap();
        json.null().unwrap();
        let header = ToolRowHeader::new(
            text("t", &ledger),
            text("literal", &ledger),
            ToolKind::Other,
            ToolStatus::Pending,
        );
        let row =
            DecodedRow::tool_without_output(header, Some(json.finish().unwrap()), None, &ledger)
                .unwrap();
        let Entry::Tool(call) = row.as_entry() else {
            panic!("tool")
        };
        assert!(call.arguments.as_ref().unwrap().as_value().is_null());
        assert!(call.result.is_none());
    }
    #[test]
    fn public_tool_json_alias_keeps_credit_after_the_row_is_gone() {
        let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 16384).unwrap();
        let baseline = ledger.live_requested_bytes();
        let mut builder = LiteralBuilder::new(&ledger).unwrap();
        let mut payload = builder.text().unwrap();
        payload.append("original tool argument").unwrap();
        builder.string(payload).unwrap();
        let header = ToolRowHeader::new(
            text("id", &ledger),
            text("title", &ledger),
            ToolKind::Other,
            ToolStatus::Pending,
        );
        let row =
            DecodedRow::tool_without_output(header, Some(builder.finish().unwrap()), None, &ledger)
                .unwrap();
        let Entry::Tool(tool) = row.as_entry() else {
            panic!("tool")
        };
        let arguments = tool.arguments.as_ref().unwrap().clone();
        let receipt = arguments.receipt().clone();
        drop(row);
        assert_eq!(
            arguments.as_value().as_str(),
            Some("original tool argument")
        );
        assert!(receipt.matches(&arguments));
        assert_eq!(
            ledger.live_requested_bytes(),
            baseline + arguments.retained_heap_bytes()
        );
        drop(arguments);
        assert_eq!(
            ledger.live_requested_bytes(),
            baseline + receipt.retained_identity_bytes().unwrap()
        );
        drop(receipt);
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }
}

#[cfg(test)]
mod identity_lifetime_tests {
    use super::*;
    use crate::model::ids::ToolId;
    #[test]
    fn escaped_tool_identifier_keeps_its_text_credit_after_the_original_row_drops() {
        let authority = crate::model::retention::RetentionAuthority::new(1024 * 1024).unwrap();
        let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
        let baseline = ledger.live_requested_bytes();
        let mut id = LiteralText::new(&ledger).unwrap();
        id.append("original retained tool identifier").unwrap();
        let mut title = LiteralText::new(&ledger).unwrap();
        title.append("original title").unwrap();
        let row = DecodedRow::tool_without_output(
            ToolRowHeader::new(id, title, ToolKind::Other, ToolStatus::Pending),
            None,
            None,
            &ledger,
        )
        .unwrap();
        let Entry::Tool(tool) = row.as_entry() else {
            panic!("tool row")
        };
        let id = tool.id.clone();
        let identity = id.retention_identity();
        let retained = id.retained_heap_bytes().unwrap();
        drop(row);
        assert_eq!(id.as_str(), "original retained tool identifier");
        assert_eq!(ledger.live_requested_bytes(), baseline + retained);
        drop(id);
        assert_eq!(
            ledger.live_requested_bytes(),
            baseline + ToolId::inline_bytes()
        );
        assert_eq!(
            crate::model::retention::graph::measure(&authority, &identity).unwrap(),
            crate::model::retention::RetentionAuthority::allocation_bytes()
                + baseline
                + ToolId::inline_bytes(),
        );
        drop(identity);
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }
}

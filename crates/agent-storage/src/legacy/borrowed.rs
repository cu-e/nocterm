//! One lossless borrowed grammar supplies legacy admission sizing and publication.
use super::{content_borrowed::optional, entries, order::RowOrderExt, prompts};
use nocterm_ai::{
    history::{
        FormatSize, HistoryFraming, HistoryShape, HistoryView, LegacySizing, LegacySizingError,
        MeasurementError, SavedPrompt, measure_json,
    },
    thread::Entry,
};
use serde::{Serialize, ser::SerializeMap};

pub(super) struct Grammar {
    rows: nocterm_ai::history::retention::LegacyRows<super::order::OrderFields>,
}
impl Grammar {
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn new(authority: &nocterm_ai::model::retention::RetentionAuthority) -> Self {
        Self {
            rows: nocterm_ai::history::retention::LegacyRows::from_untracked(authority, Vec::new()),
        }
    }
    pub(super) fn admitted(
        ledger: &nocterm_ai::model::json::deferred::DecodeLedger,
        capacity: usize,
    ) -> Result<Self, nocterm_ai::model::json::deferred::JsonDecodeError> {
        Ok(Self {
            rows: nocterm_ai::history::retention::LegacyRows::admitted(ledger, capacity)?,
        })
    }
    pub(super) fn record_admitted(
        &mut self,
        order: Option<super::order::RowOrder>,
    ) -> Result<(), nocterm_ai::model::json::deferred::JsonDecodeError> {
        if let Some(order) = order {
            self.rows.push_admitted(order)?;
        }
        Ok(())
    }
    pub(super) fn retained_admitted(
        self,
        ledger: &nocterm_ai::model::json::deferred::DecodeLedger,
    ) -> Result<
        nocterm_ai::history::retention::LegacyFormat,
        nocterm_ai::model::json::deferred::JsonDecodeError,
    > {
        let source = self.rows.source();
        nocterm_ai::history::retention::retain_legacy_admitted(ledger, self, source)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn retained(
        self,
        authority: &nocterm_ai::model::retention::RetentionAuthority,
    ) -> nocterm_ai::history::retention::LegacyFormat {
        let source = self.rows.source();
        nocterm_ai::history::retention::retain_legacy(authority, self, source)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn record(
        &mut self,
        order: Option<super::order::RowOrder>,
    ) -> Result<(), nocterm_ai::model::retention::RetentionError> {
        if let Some(order) = order {
            self.rows.push(order)?;
        }
        Ok(())
    }
    pub(super) fn managed_order(
        &self,
        row: &nocterm_ai::history::physical::ToolSlot,
    ) -> Option<&super::order::RowOrder> {
        self.rows
            .iter()
            .find(|order| order.identity().matches_managed(row))
    }
    pub(super) fn order(
        &self,
        row: &nocterm_ai::model::tools::ToolRecord,
    ) -> Option<&super::order::RowOrder> {
        self.rows.iter().find(|order| order.matches(row))
    }
}
impl nocterm_ai::history::HistoryJsonRetention for Grammar {
    fn visit_json_allocations<'a>(
        &'a self,
        visit: &mut dyn FnMut(
            nocterm_ai::model::json::RetainedAllocation<'a>,
        ) -> Result<(), nocterm_ai::history::RetentionError>,
    ) -> Result<(), nocterm_ai::history::RetentionError> {
        for row in self.rows.iter() {
            row.visit_json_allocations(visit)?;
        }
        Ok(())
    }
}
fn measured(value: &impl Serialize) -> Result<FormatSize, LegacySizingError> {
    measure_json(value).map_err(|error| match error {
        MeasurementError::Json(error) => LegacySizingError::Json(error),
        MeasurementError::ArithmeticOverflow => LegacySizingError::ArithmeticOverflow,
        MeasurementError::Retention(error) => LegacySizingError::Retention(error),
        MeasurementError::Formatting(error) => LegacySizingError::Formatting(error),
    })
}
impl LegacySizing for Grammar {
    fn managed_entry(
        &self,
        row: &nocterm_ai::history::physical::RowSlot,
    ) -> Result<FormatSize, LegacySizingError> {
        super::managed::check_row(row).map_err(LegacySizingError::UnsupportedRecord)?;
        measured(&super::managed::Row(row, self))
    }
    fn managed_prompt(
        &self,
        prompt: &nocterm_ai::history::physical::PromptSlot,
    ) -> Result<FormatSize, LegacySizingError> {
        super::managed::check_prompt(prompt).map_err(LegacySizingError::UnsupportedRecord)?;
        measured(&super::managed::Prompt(prompt))
    }
    fn managed_measure(
        &self,
        snapshot: &nocterm_ai::history::physical::SnapshotOwner,
    ) -> Result<FormatSize, LegacySizingError> {
        let rows = snapshot.rows();
        for row in &rows[rows
            .len()
            .saturating_sub(nocterm_ai::history::MAX_SAVED_ENTRIES)..]
        {
            super::managed::check_row(row).map_err(LegacySizingError::UnsupportedRecord)?;
        }
        for prompt in snapshot.composer().queue() {
            super::managed::check_prompt(prompt).map_err(LegacySizingError::UnsupportedRecord)?;
        }
        measured(&super::managed::Document(snapshot, self))
    }
    fn retained_heap_bytes(&self) -> Result<usize, LegacySizingError> {
        let mut total = self
            .rows
            .capacity()
            .checked_mul(std::mem::size_of::<super::order::RowOrder>())
            .and_then(|bytes| {
                bytes.checked_add(nocterm_ai::history::retention::LegacyRows::<
                    super::order::OrderFields,
                >::inline_bytes())
            })
            .ok_or(LegacySizingError::ArithmeticOverflow)?;
        for row in self.rows.iter() {
            total = total
                .checked_add(
                    row.retained_heap_bytes()
                        .ok_or(LegacySizingError::ArithmeticOverflow)?,
                )
                .ok_or(LegacySizingError::ArithmeticOverflow)?;
        }
        Ok(total)
    }
    fn entry(&self, row: &Entry) -> Result<FormatSize, LegacySizingError> {
        entries::check(row).map_err(LegacySizingError::UnsupportedRecord)?;
        measured(&entries::borrowed(row, self))
    }
    fn prompt(&self, prompt: &SavedPrompt) -> Result<FormatSize, LegacySizingError> {
        prompts::check(prompt).map_err(LegacySizingError::UnsupportedRecord)?;
        measured(&prompts::Borrowed(prompt))
    }
    fn framing(
        &self,
        frame: HistoryFraming<'_>,
        shape: HistoryShape,
    ) -> Result<FormatSize, LegacySizingError> {
        // Empty arrays contribute exactly their container/bracket cost. Actual
        // cached rows/prompts and commas are composed by the domain admission.
        measured(&Framing(frame, shape))
    }
    fn measure(&self, candidate: HistoryView<'_>) -> Result<FormatSize, LegacySizingError> {
        check(candidate)?;
        measured(&Borrowed(candidate, self))
    }
}
pub(super) fn check(value: HistoryView<'_>) -> Result<(), LegacySizingError> {
    for row in value.entries() {
        entries::check(row).map_err(LegacySizingError::UnsupportedRecord)?;
    }
    for index in 0..value.prompts().len() {
        let prompt = value.prompts().get(index).ok_or_else(|| {
            LegacySizingError::Json(<serde_json::Error as serde::ser::Error>::custom(
                "prepared prompt missing",
            ))
        })?;
        prompts::check(prompt).map_err(LegacySizingError::UnsupportedRecord)?;
    }
    Ok(())
}
pub(super) struct Borrowed<'a>(pub(super) HistoryView<'a>, pub(super) &'a Grammar);
impl crate::writing::managed_writer::ManagedWire for Borrowed<'_> {}
impl Serialize for Borrowed<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        document(
            self.0.framing(),
            &entries::Rows(self.0.entries(), self.1),
            (!self.0.prompts().is_empty()).then_some(prompts::Queue(self.0.prompts())),
            serializer,
        )
    }
}
struct Framing<'a>(HistoryFraming<'a>, HistoryShape);
impl Serialize for Framing<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        document(
            self.0,
            &[] as &[u8],
            (self.1.prompt_count != 0).then_some(&[] as &[u8]),
            serializer,
        )
    }
}
fn document<S: serde::Serializer, R: Serialize + ?Sized, Q: Serialize>(
    frame: HistoryFraming<'_>,
    rows: &R,
    queue: Option<Q>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let metadata = frame.metadata;
    let resume = frame.resume;
    let mut map = serializer.serialize_map(None)?;
    map.serialize_entry("version", &1u32)?;
    map.serialize_entry("id", frame.id)?;
    map.serialize_entry("agent_id", &metadata.agent_id)?;
    optional(&mut map, "title", &metadata.title)?;
    optional(&mut map, "name", &metadata.name)?;
    if metadata.pinned {
        map.serialize_entry("pinned", &true)?;
    }
    optional(&mut map, "session_id", &resume.session_id)?;
    optional(&mut map, "workdir", &resume.workdir)?;
    optional(&mut map, "model", &metadata.model)?;
    if !resume.config.is_empty() {
        map.serialize_entry("config", &resume.config)?;
    }
    optional(&mut map, "mode", &resume.mode)?;
    if resume.pending_history {
        map.serialize_entry("pending_history", &true)?;
    }
    if let Some(value) = frame.draft {
        map.serialize_entry("draft", value)?;
    }
    map.serialize_entry("updated", &metadata.updated)?;
    map.serialize_entry("entries", rows)?;
    if let Some(value) = queue {
        map.serialize_entry("queue", &value)?;
    }
    if !frame.attachments.is_empty() {
        map.serialize_entry("attachments", frame.attachments)?;
    }
    if !frame.times.is_empty() {
        map.serialize_entry("times", frame.times)?;
    }
    map.end()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn real_borrowed_legacy_framing_plus_rows_and_queue_equals_whole_cost() {
        let authority = nocterm_ai::model::retention::RetentionAuthority::new(
            nocterm_ai::history::MAX_RESIDENT_BYTES,
        )
        .unwrap();
        let id = "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c";
        let bytes = serde_json::to_vec(&json!({
            "version":1,"id":id,"agent_id":"original provider","updated":17,
            "title":"λ","name":"literal name","pinned":true,
            "session_id":"original-session","workdir":"/original","model":"original model",
            "mode":"original mode","pending_history":true,"draft":"literal draft",
            "entries":[{"kind":"agent","value":"one"},{"kind":"thought","value":"two"}],
            "times":[11,12],"attachments":[{"kind":"group","id":"original-group"}],
            "queue":[{"id":7,"text":"queued one"},{"id":8,"text":"queued two"}]
        }))
        .unwrap();
        let raw = super::super::wire::decode(&bytes, id).unwrap();
        let expected = serde_json::to_vec(&raw).unwrap();
        let (owned, grammar) = raw.into_owned_ordered(&authority).unwrap();
        let view = owned.view().unwrap();
        let actual = serde_json::to_vec(&Borrowed(view, &grammar)).unwrap();
        assert_eq!(actual, expected);
        let whole = grammar.measure(view).unwrap();
        assert_eq!(whole.encoded_bytes, actual.len());
        assert_eq!(
            whole.structural_bytes,
            nocterm_ai::history::structural_bytes(&actual).unwrap()
        );
        let mut composed = grammar.framing(view.framing(), view.shape()).unwrap();
        for row in view.entries() {
            let cost = grammar.entry(row).unwrap();
            composed.encoded_bytes += cost.encoded_bytes;
            composed.structural_bytes += cost.structural_bytes;
        }
        for index in 0..view.prompts().len() {
            let cost = grammar.prompt(view.prompts().get(index).unwrap()).unwrap();
            composed.encoded_bytes += cost.encoded_bytes;
            composed.structural_bytes += cost.structural_bytes;
        }
        composed.encoded_bytes +=
            view.entries().len().saturating_sub(1) + view.prompts().len().saturating_sub(1);
        assert_eq!(composed, whole);
        // This verifies the exact grammar/cost decomposition, not filesystem or
        // public runtime mutation/admission behavior, which remains unwired.
    }
}

#[cfg(test)]
mod retention_tests;

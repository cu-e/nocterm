//! What the person's own agents consumed, per model.

use std::collections::BTreeMap;

/// One day's counters of one agent and model, as the hub reports them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageSample {
    /// Model id; empty for rows recorded before the hub counted per model.
    pub model: String,
    /// Runs started.
    pub runs: u64,
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Spend in millionths of the hub's currency unit.
    pub spend_micros: u64,
}

/// The totals of one model over a period.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelUsage {
    /// Model id; empty when the hub did not attribute the rows to a model.
    pub model: String,
    /// Runs started.
    pub runs: u64,
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Spend in millionths of the hub's currency unit.
    pub spend_micros: u64,
}

impl ModelUsage {
    /// Sums the samples per model. The largest spender comes first, then the model id; a
    /// counter that would overflow stays at the maximum.
    pub fn by_model(samples: impl IntoIterator<Item = UsageSample>) -> Vec<Self> {
        let mut totals: BTreeMap<String, Self> = BTreeMap::new();
        for sample in samples {
            let total = totals.entry(sample.model.clone()).or_insert_with(|| Self {
                model: sample.model.clone(),
                runs: 0,
                input_tokens: 0,
                output_tokens: 0,
                spend_micros: 0,
            });
            total.runs = total.runs.saturating_add(sample.runs);
            total.input_tokens = total.input_tokens.saturating_add(sample.input_tokens);
            total.output_tokens = total.output_tokens.saturating_add(sample.output_tokens);
            total.spend_micros = total.spend_micros.saturating_add(sample.spend_micros);
        }
        let mut rows: Vec<Self> = totals.into_values().collect();
        rows.sort_by(|a, b| {
            b.spend_micros
                .cmp(&a.spend_micros)
                .then_with(|| a.model.cmp(&b.model))
        });
        rows
    }

    /// The model as shown: the id, or a note for rows without one.
    pub fn label(&self) -> &str {
        if self.model.is_empty() {
            "not attributed"
        } else {
            &self.model
        }
    }
}

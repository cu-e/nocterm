use super::extensions::Extensions;

/// How full the model's context is.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextUsage {
    pub used: u64,
    pub size: u64,
    pub cost: Option<Cost>,
    pub meta: Option<Extensions>,
}
impl ContextUsage {
    pub fn new(used: u64, size: u64) -> Self {
        Self {
            used,
            size,
            cost: None,
            meta: None,
        }
    }
    pub fn meta(mut self, meta: impl Into<Option<Extensions>>) -> Self {
        self.meta = meta.into();
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cost {
    pub amount: f64,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenTotals {
    pub total: u64,
    pub input: u64,
    pub output: u64,
    pub thought: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
}
impl TokenTotals {
    pub fn new(total: u64, input: u64, output: u64) -> Self {
        Self {
            total,
            input,
            output,
            thought: None,
            cache_read: None,
            cache_write: None,
        }
    }
}

use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextUsage {
    pub used: u64,
    pub size: u64,
    pub cost: Option<Cost>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    pub amount: f64,
    pub currency: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenTotals {
    pub total: u64,
    pub input: u64,
    pub output: u64,
    pub thought: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
}

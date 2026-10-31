//! Policy spend ceilings the runner holds on top of the commands' allowances
//! (ADR 0127). Every quote of a model call is charged at its worst-case cost;
//! a call that would pass the run or chat ceiling is refused before it reaches
//! the broker. The totals live in memory only and are conservative (a quote
//! that is never reserved still counts), so they can only refuse sooner; the
//! broker remains the spend authority.
use nocterm_cloud_model::{
    engine::ModelCallRequest,
    identity::{CloudChatId, RunId},
    inference::{InferenceError, ModelPricing, ModelQuote, ModelSelection},
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, PoisonError},
};

/// Totals kept per run and per chat; the oldest entries are forgotten first.
const TRACKED: usize = 4096;

struct Totals<K: Ord + Copy> {
    spent: BTreeMap<K, u64>,
    order: VecDeque<K>,
}
impl<K: Ord + Copy> Totals<K> {
    fn new() -> Self {
        Self {
            spent: BTreeMap::new(),
            order: VecDeque::new(),
        }
    }
    fn spent(&self, key: K) -> u64 {
        self.spent.get(&key).copied().unwrap_or(0)
    }
    fn add(&mut self, key: K, micros: u64) {
        if !self.spent.contains_key(&key) {
            if self.order.len() >= TRACKED
                && let Some(oldest) = self.order.pop_front()
            {
                self.spent.remove(&oldest);
            }
            self.order.push_back(key);
        }
        let total = self.spent.entry(key).or_default();
        *total = total.saturating_add(micros);
    }
}
struct Ledger {
    runs: Totals<(CloudChatId, RunId)>,
    chats: Totals<CloudChatId>,
}

/// The ceilings of one policy, shared by the pricing of every model.
pub(crate) struct SpendCeilings {
    per_run: Option<u64>,
    per_chat: Option<u64>,
    ledger: Mutex<Ledger>,
}
impl SpendCeilings {
    /// `None` when the policy sets no spend ceiling.
    pub(crate) fn new(per_run: Option<u64>, per_chat: Option<u64>) -> Option<Arc<Self>> {
        (per_run.is_some() || per_chat.is_some()).then(|| {
            Arc::new(Self {
                per_run,
                per_chat,
                ledger: Mutex::new(Ledger {
                    runs: Totals::new(),
                    chats: Totals::new(),
                }),
            })
        })
    }
    fn charge(&self, request: &ModelCallRequest, micros: u64) -> Result<(), InferenceError> {
        let run = (request.run().chat(), request.run().run());
        let chat = request.run().chat();
        let mut ledger = self.ledger.lock().unwrap_or_else(PoisonError::into_inner);
        let over = |ceiling: Option<u64>, spent: u64| {
            ceiling.is_some_and(|ceiling| spent.saturating_add(micros) > ceiling)
        };
        if over(self.per_run, ledger.runs.spent(run))
            || over(self.per_chat, ledger.chats.spent(chat))
        {
            return Err(InferenceError::BudgetExceeded);
        }
        ledger.runs.add(run, micros);
        ledger.chats.add(chat, micros);
        Ok(())
    }
}

/// A model's pricing that also charges the policy's spend ceilings.
pub(crate) struct CeilingPricing {
    inner: Arc<dyn ModelPricing>,
    ceilings: Arc<SpendCeilings>,
}
impl CeilingPricing {
    pub(crate) fn new(inner: Arc<dyn ModelPricing>, ceilings: Arc<SpendCeilings>) -> Self {
        Self { inner, ceilings }
    }
}
impl ModelPricing for CeilingPricing {
    fn selection(&self) -> &ModelSelection {
        self.inner.selection()
    }
    fn quote(&self, request: &ModelCallRequest) -> Result<ModelQuote, InferenceError> {
        let quote = self.inner.quote(request)?;
        self.ceilings.charge(request, quote.spend_micros())?;
        Ok(quote)
    }
}

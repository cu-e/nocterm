//! Native SDK allocations use the one actual connection's original source loan.
use agent_client_protocol::managed::{AllocationKind, Budget};
use nocterm_ai::{
    history::source::{NativeAllocation, SourceContext},
    model::json::deferred::{DecodeLedger, JsonDecodeError},
};
use std::alloc::Layout;

#[derive(Clone)]
pub(crate) struct SourceBudget {
    context: SourceContext,
}
impl SourceBudget {
    pub(crate) fn new(context: SourceContext) -> Self {
        Self { context }
    }
    pub(crate) fn context(&self) -> &SourceContext {
        &self.context
    }
    pub(crate) fn ledger(&self) -> &DecodeLedger {
        self.context.ledger()
    }
}
impl Budget for SourceBudget {
    type Credit = NativeAllocation;
    type Error = JsonDecodeError;
    fn same_source(&self, other: &Self) -> bool {
        self.context.same_source(&other.context)
    }
    fn reserve(&self, layout: Layout, _kind: AllocationKind) -> Result<Self::Credit, Self::Error> {
        self.context.reserve_native(layout)
    }
}

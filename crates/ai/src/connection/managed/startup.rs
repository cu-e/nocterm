//! One admitted immutable startup graph can be aliased by native preparation.
use crate::{
    connection::ManagedStartupFacts,
    history::source::{NativeAllocation, SourceContext},
    model::{
        json::{deferred::JsonDecodeError, lifetime::Shared},
        native::NativeOwned,
    },
};
use std::alloc::Layout;
struct StartupBody {
    original: NativeOwned<ManagedStartupFacts>,
    allocation: NativeAllocation,
}
/// Cloning shares the original paid launch/workdir facts and allocates nothing.
/// The handle owns no Source lifetime, so a stopped command cannot retain a loan
/// through an immutable startup/cancellation dependency cycle.
#[derive(Clone)]
pub struct ManagedStartupHandle(Shared<StartupBody>);
impl ManagedStartupHandle {
    pub fn new(
        context: &SourceContext,
        original: NativeOwned<ManagedStartupFacts>,
    ) -> Result<Self, JsonDecodeError> {
        if !original.belongs_to_ledger(context.ledger()) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        let layout = Layout::from_size_align(
            crate::model::allocations::shared_inline_bytes::<StartupBody>(),
            std::mem::align_of::<StartupBody>().max(std::mem::align_of::<usize>()),
        )
        .map_err(|_| JsonDecodeError::Arithmetic)?;
        let allocation = context.reserve_native(layout)?;
        Ok(Self(Shared::new(StartupBody {
            original,
            allocation,
        })))
    }
    pub fn facts(&self) -> &ManagedStartupFacts {
        self.0.original.get()
    }
    pub fn belongs_to_source(&self, context: &SourceContext) -> bool {
        self.0.allocation.source().same_source(context)
    }
    pub fn same_startup(&self, other: &Self) -> bool {
        std::ptr::eq(self.0.get(), other.0.get())
    }
}

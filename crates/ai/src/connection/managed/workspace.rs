//! Immutable aliases retain the actual paid resolved workspace exactly once.
use crate::{
    history::source::{NativeAllocation, SourceContext},
    model::{
        allocations::shared_inline_bytes,
        json::{deferred::JsonDecodeError, lifetime::Shared},
        native::NativeOwned,
    },
    workspace::PreparedWorkspace,
};
use std::alloc::Layout;
struct WorkspaceBody {
    original: NativeOwned<PreparedWorkspace>,
    allocation: NativeAllocation,
}
#[derive(Clone)]
pub struct ManagedWorkspaceHandle(Shared<WorkspaceBody>);
impl ManagedWorkspaceHandle {
    pub fn new(
        context: &SourceContext,
        original: NativeOwned<PreparedWorkspace>,
    ) -> Result<Self, JsonDecodeError> {
        if !original.belongs_to_ledger(context.ledger()) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        let layout = Layout::from_size_align(
            shared_inline_bytes::<WorkspaceBody>(),
            std::mem::align_of::<WorkspaceBody>().max(std::mem::align_of::<usize>()),
        )
        .map_err(|_| JsonDecodeError::Arithmetic)?;
        let allocation = context.reserve_native(layout)?;
        Ok(Self(Shared::new(WorkspaceBody {
            original,
            allocation,
        })))
    }
    pub fn workspace(&self) -> &PreparedWorkspace {
        self.0.original.get()
    }
    pub fn belongs_to_source(&self, context: &SourceContext) -> bool {
        self.0.allocation.source().same_source(context)
    }
    pub fn same_workspace(&self, other: &Self) -> bool {
        std::ptr::eq(self.0.get(), other.0.get())
    }
}

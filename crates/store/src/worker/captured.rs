//! One App-free final sequence: join normal work, record prefixes, run owned operations.
use super::DiskOperation;
use crate::StoreError;
use futures::future::BoxFuture;
use nocterm_core::persist::WriteGate;

pub(crate) trait CapturedPolicy: Send + 'static {
    type Physical: Send + 'static;
    type Output: Send + 'static;
    fn gate(&self) -> WriteGate;
    fn joined(&mut self);
    fn next_operation(&mut self) -> BoxFuture<'_, Option<DiskOperation<Self::Physical>>>;
    fn apply_outcome(&mut self, outcome: Result<Self::Physical, StoreError>);
    fn finish(self) -> Self::Output;
}
pub(crate) fn run<P: CapturedPolicy>(mut policy: P) -> BoxFuture<'static, P::Output> {
    Box::pin(async move {
        let gate = policy.gate();
        let _guard = gate.final_write().await;
        policy.joined();
        while let Some(operation) = policy.next_operation().await {
            let outcome = operation.execute().await;
            policy.apply_outcome(outcome);
        }
        policy.finish()
    })
}

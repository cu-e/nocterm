//! Bounded run-local once claims supplement the authoritative durable dispatch ledger.
use nocterm_cloud_model::{
    identity::IntentId,
    run::DispatchPermit,
    tool::{ScopedToolRequest, ToolExecutionError, ToolExecutionResult},
};
use std::collections::BTreeMap;

const MAX_DISPATCHES: usize = 1024;
struct DispatchObservation {
    permit: DispatchPermit,
    result: Option<Result<ToolExecutionResult, ToolExecutionError>>,
}
#[derive(Default)]
pub(super) struct DispatchBook(BTreeMap<IntentId, DispatchObservation>);
impl DispatchBook {
    pub(super) fn claim(
        &mut self,
        request: &ScopedToolRequest,
    ) -> Result<Option<Result<ToolExecutionResult, ToolExecutionError>>, ToolExecutionError> {
        let id = request.permit().intent().id();
        if let Some(observation) = self.0.get(&id) {
            if &observation.permit != request.permit() {
                return Err(ToolExecutionError::ArgumentMismatch);
            }
            return Ok(Some(
                observation
                    .result
                    .clone()
                    .unwrap_or(Err(ToolExecutionError::OutcomeUnknown)),
            ));
        }
        if self.0.len() >= MAX_DISPATCHES {
            return Err(ToolExecutionError::Capacity);
        }
        self.0.insert(
            id,
            DispatchObservation {
                permit: request.permit().clone(),
                result: None,
            },
        );
        Ok(None)
    }
    pub(super) fn finish(
        &mut self,
        id: IntentId,
        result: Result<ToolExecutionResult, ToolExecutionError>,
    ) {
        if let Some(observation) = self.0.get_mut(&id) {
            observation.result = Some(result);
        }
    }
}

//! Original run authority metadata without retaining the admitted prompt payload.
use nocterm_cloud_model::{
    coordinator::{PromptAllowance, PromptRequest, RunDestination},
    inference::ModelSelection,
};

pub(super) struct RunContext {
    selection: ModelSelection,
    destination: RunDestination,
    allowance: PromptAllowance,
}
impl From<&PromptRequest> for RunContext {
    fn from(prompt: &PromptRequest) -> Self {
        Self {
            selection: prompt.selection().clone(),
            destination: prompt.destination().clone(),
            allowance: prompt.allowance(),
        }
    }
}
impl RunContext {
    pub(super) fn selection(&self) -> &ModelSelection {
        &self.selection
    }
    pub(super) fn destination(&self) -> &RunDestination {
        &self.destination
    }
    pub(super) fn allowance(&self) -> PromptAllowance {
        self.allowance
    }
}

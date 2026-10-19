//! Result preparation retains the original native error and semantic refusal.
use super::*;
use nocterm_ai::history::source::{SourceContext, SourceErrorSlot, SourceSemanticRefusal};
use nocterm_ai::model::json::deferred::JsonDecodeError;
pub(super) struct OpenedReply {
    copy: SourceErrorSlot<JsonDecodeError>,
    certification: SourceErrorSlot<SourceSemanticRefusal<SessionOpened>>,
    context: SourceContext,
}
impl OpenedReply {
    pub(super) fn new(context: &SourceContext) -> Result<Self, NativeAllocationError> {
        Ok(Self {
            copy: context.prepare_error(0)?,
            certification: context.prepare_error(0)?,
            context: context.clone(),
        })
    }
    pub(super) fn finish(
        self,
        commands: &Commands,
        session: &str,
    ) -> Result<ManagedOpened, ManagedCommandError> {
        let native = commands
            .config
            .opened_native(session, &self.context)
            .map_err(|error| ManagedCommandError::Native(self.copy.capture(error)))?;
        self.context
            .certify_opened(native)
            .map_err(|error| ManagedCommandError::Native(self.certification.capture(error)))
    }
}

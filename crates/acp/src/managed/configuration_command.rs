//! Legacy offered-model routing commits independent cache and delivered owners.
use super::{
    outgoing, results,
    rpc::{Cause, Rpc},
};
use agent_client_protocol::managed::Source;
use nocterm_ai::{
    history::source::{ManagedCommandError, ManagedSourceFuture},
    model::{
        controls::ConfigOption, json::deferred::NativeAllocationError, native::NativeOwned,
        session::ConfigurationSelection,
    },
};
pub(super) struct Configuration {
    rpc: Rpc,
}
impl Configuration {
    pub(super) fn new(rpc: Rpc) -> Self {
        Self { rpc }
    }
    pub(super) fn execute(
        self,
        request: &ConfigurationSelection,
    ) -> Result<
        ManagedSourceFuture<Result<NativeOwned<Vec<ConfigOption>>, ManagedCommandError>>,
        NativeAllocationError,
    > {
        let prepared = self.prepare(request);
        let (session, selection, envelope) = match prepared {
            Ok(parts) => parts,
            Err(error) => return self.rpc.operation(Err(error), false, |_, _| unreachable!()),
        };
        self.rpc
            .operation(Ok(envelope), false, move |value, models| {
                if let Some(selection) = selection {
                    Ok(models.selected(session.text(), Some(selection.as_str()))?)
                } else {
                    let options = results::configuration(value)?;
                    Ok(models.retain(session.text(), options)?)
                }
            })
    }
    fn prepare(&self, request: &ConfigurationSelection) -> Result<Prepared, Cause> {
        let models = self
            .rpc
            .models()
            .get()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let selected = match &request.value {
            nocterm_ai::session_config::ConfigValue::Value(value) => Some(value.as_str()),
            _ => None,
        };
        let selection =
            models.selection(request.session.as_str(), request.option.as_str(), selected)?;
        drop(models);
        let session = Source::copy(request.session.as_str(), self.rpc.budget().clone())?;
        let id = self.rpc.id()?;
        let envelope = match &selection {
            Some(selection) => {
                outgoing::model(self.rpc.budget(), &id, session.text(), selection.as_str())?
            }
            None => outgoing::configuration(self.rpc.budget(), &id, request)?,
        };
        Ok((session, selection, envelope))
    }
}
type Prepared = (
    Source<super::budget::SourceBudget>,
    Option<NativeOwned<String>>,
    agent_client_protocol::managed::Envelope<super::budget::SourceBudget>,
);

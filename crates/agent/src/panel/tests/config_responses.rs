//! Configuration replies extend the existing shared agent command fixture.
use super::*;
#[derive(Default)]
pub(super) struct ConfigResponses {
    options: Mutex<Vec<ConfigOption>>,
    modes: Mutex<Option<Modes>>,
    requests: Mutex<Vec<ConfigurationSelection>>,
}
impl ConfigResponses {
    pub(super) fn set_options(&self, options: Vec<ConfigOption>) {
        *self.options.lock().unwrap() = options;
    }
    pub(super) fn set_modes(&self, modes: Modes) {
        *self.modes.lock().unwrap() = Some(modes);
    }
    pub(super) fn requests(&self) -> Vec<ConfigurationSelection> {
        self.requests.lock().unwrap().clone()
    }
    pub(super) fn opened_native(
        &self,
        session: &str,
        context: &nocterm_ai::history::source::SourceContext,
    ) -> Result<
        nocterm_ai::model::native::NativeOwned<SessionOpened>,
        nocterm_ai::model::json::deferred::JsonDecodeError,
    > {
        let modes = self.modes.lock().unwrap();
        let options = self.options.lock().unwrap();
        nocterm_ai::model::native::NativeCopies::new(context.ledger()).opened_fields(
            session,
            modes.as_ref(),
            (!options.is_empty()).then_some(options.as_slice()),
        )
    }
    pub(super) fn choose_native(
        &self,
        request: &ConfigurationSelection,
        context: &nocterm_ai::history::source::SourceContext,
    ) -> Result<
        nocterm_ai::model::native::NativeOwned<Vec<ConfigOption>>,
        nocterm_ai::model::json::deferred::JsonDecodeError,
    > {
        let mut options = self.options.lock().unwrap();
        nocterm_ai::session_config::choose(&mut options, request.option.as_str(), &request.value);
        self.requests.lock().unwrap().push(request.clone());
        nocterm_ai::model::native::NativeCopies::new(context.ledger())
            .configurations(&options, None)
    }
}

//! Configuration replies extend the existing shared agent command fixture.
use super::*;
#[derive(Default)]
pub(super) struct ConfigResponses {
    options: Mutex<Vec<acp::SessionConfigOption>>,
    modes: Mutex<Option<acp::SessionModeState>>,
    requests: Mutex<Vec<acp::SetSessionConfigOptionRequest>>,
}
impl ConfigResponses {
    pub(super) fn set_options(&self, options: Vec<acp::SessionConfigOption>) {
        *self.options.lock().unwrap() = options;
    }
    pub(super) fn set_modes(&self, modes: acp::SessionModeState) {
        *self.modes.lock().unwrap() = Some(modes);
    }
    pub(super) fn options(&self) -> Vec<acp::SessionConfigOption> {
        self.options.lock().unwrap().clone()
    }
    pub(super) fn modes(&self) -> Option<acp::SessionModeState> {
        self.modes.lock().unwrap().clone()
    }
    pub(super) fn requests(&self) -> Vec<acp::SetSessionConfigOptionRequest> {
        self.requests.lock().unwrap().clone()
    }
    pub(super) fn choose(
        &self,
        request: acp::SetSessionConfigOptionRequest,
    ) -> Vec<acp::SessionConfigOption> {
        let mut options = self.options.lock().unwrap();
        if let Some(value) = nocterm_ai::session_config::ConfigValue::from_acp(&request.value) {
            nocterm_ai::session_config::choose(&mut options, &request.config_id.0, &value);
        }
        self.requests.lock().unwrap().push(request);
        options.clone()
    }
}

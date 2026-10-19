//! Configuration replies extend the existing shared agent command fixture.
use super::*;
use nocterm_ai::model::controls::{ConfigOption, Modes};
use nocterm_ai::model::session::SetConfigOption;
#[derive(Default)]
pub(super) struct ConfigResponses {
    options: Mutex<Vec<ConfigOption>>,
    modes: Mutex<Option<Modes>>,
    requests: Mutex<Vec<SetConfigOption>>,
}
impl ConfigResponses {
    pub(super) fn set_options(&self, options: Vec<ConfigOption>) {
        *self.options.lock().unwrap() = options;
    }
    pub(super) fn set_modes(&self, modes: Modes) {
        *self.modes.lock().unwrap() = Some(modes);
    }
    pub(super) fn options(&self) -> Vec<ConfigOption> {
        self.options.lock().unwrap().clone()
    }
    pub(super) fn modes(&self) -> Option<Modes> {
        self.modes.lock().unwrap().clone()
    }
    pub(super) fn requests(&self) -> Vec<SetConfigOption> {
        self.requests.lock().unwrap().clone()
    }
    pub(super) fn choose(&self, request: SetConfigOption) -> Vec<ConfigOption> {
        let mut options = self.options.lock().unwrap();
        let value = nocterm_ai::session_config::ConfigValue::from_setting(&request.setting);
        nocterm_ai::session_config::choose(&mut options, request.config.as_str(), &value);
        self.requests.lock().unwrap().push(request);
        options.clone()
    }
}

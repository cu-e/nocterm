use gpui::App;
use std::rc::Rc;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountryPolicySnapshot {
    enabled: bool,
    published_revision: u64,
}
impl CountryPolicySnapshot {
    pub fn new(enabled: bool, published_revision: u64) -> Self {
        Self {
            enabled,
            published_revision,
        }
    }
    pub fn enabled(self) -> bool {
        self.enabled
    }
}
pub trait CountryPolicySource {
    fn current(&self, cx: &App) -> CountryPolicySnapshot;
}
pub(super) struct DetectionPolicy {
    source: Rc<dyn CountryPolicySource>,
    last: CountryPolicySnapshot,
}
impl DetectionPolicy {
    pub(super) fn new(source: Rc<dyn CountryPolicySource>, cx: &App) -> Self {
        let last = source.current(cx);
        Self { source, last }
    }
    pub(super) fn read(&mut self, cx: &App) -> CountryPolicySnapshot {
        self.last = self.source.current(cx);
        self.last
    }
}

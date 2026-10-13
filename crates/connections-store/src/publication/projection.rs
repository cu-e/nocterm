//! One immutable committed projection carrier, separate from leased profile persistence.
use gpui::Context;
use nocterm_connections_model::profile_authority::ProfilePublication;
pub struct ProfilePublications {
    current: ProfilePublication,
}
impl ProfilePublications {
    pub(crate) fn new(current: ProfilePublication) -> Self {
        Self { current }
    }
    pub fn current(&self) -> &ProfilePublication {
        &self.current
    }
    pub(crate) fn publish(&mut self, current: ProfilePublication, cx: &mut Context<Self>) {
        self.current = current;
        cx.notify();
    }
}

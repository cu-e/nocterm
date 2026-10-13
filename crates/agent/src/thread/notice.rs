//! Status text and its error emphasis have one presentation owner.
pub(super) struct ThreadNotice {
    text: String,
    error: bool,
}
impl Default for ThreadNotice {
    fn default() -> Self {
        Self {
            text: "Ready — send a message to start the agent".into(),
            error: false,
        }
    }
}
impl ThreadNotice {
    pub(super) fn text(&self) -> String {
        self.text.clone()
    }
    pub(super) fn is_error(&self) -> bool {
        self.error
    }
    /// A text-only transition deliberately retains existing error emphasis.
    pub(super) fn present(&mut self, text: String) {
        self.text = text;
    }
    pub(super) fn clear_text(&mut self) {
        self.text.clear();
    }
    pub(super) fn mark_error(&mut self) {
        self.error = true;
    }
    pub(super) fn clear_error(&mut self) {
        self.error = false;
    }
}

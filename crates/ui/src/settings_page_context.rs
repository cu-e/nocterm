//! A settings Item shares diagnostic presentation without sharing its mutable view.
use gpui_kit::{App, AppContext as _, Entity, ScrollHandle, SharedString, Subscription, Task};
use nocterm_store::{SettingsHandle, StoreError};

struct FeedbackState {
    error: Option<SharedString>,
}
#[derive(Clone)]
pub struct SettingsPageFeedback(Entity<FeedbackState>);
impl SettingsPageFeedback {
    pub fn new(cx: &mut App) -> Self {
        Self(cx.new(|_| FeedbackState { error: None }))
    }
    pub fn error(&self, cx: &App) -> Option<SharedString> {
        self.0.read(cx).error.clone()
    }
    pub fn observe(
        &self,
        cx: &mut App,
        mut changed: impl FnMut(&mut App) + 'static,
    ) -> Subscription {
        cx.observe(&self.0, move |_, cx| changed(cx))
    }
    pub fn report(&self, task: Task<Result<u64, StoreError>>, cx: &mut App) {
        self.0.update(cx, |_, cx| {
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |this, cx| {
                    this.error = result
                        .err()
                        .map(|error| format!("Could not save settings: {error}").into());
                    cx.notify();
                });
            })
            .detach();
        });
    }
}
#[derive(Clone)]
pub struct SettingsPageContext {
    settings: SettingsHandle,
    feedback: SettingsPageFeedback,
    scroll: ScrollHandle,
}
impl SettingsPageContext {
    pub fn new(settings: SettingsHandle, feedback: SettingsPageFeedback) -> Self {
        Self {
            settings,
            feedback,
            scroll: ScrollHandle::new(),
        }
    }
    /// The context with the handle of the scroll the host shows the page in.
    pub fn with_scroll(mut self, scroll: ScrollHandle) -> Self {
        self.scroll = scroll;
        self
    }
    /// The scroll of the host that shows the page, which a part of the page can scroll to.
    pub fn scroll(&self) -> &ScrollHandle {
        &self.scroll
    }
    pub fn settings(&self) -> &SettingsHandle {
        &self.settings
    }
    pub fn feedback(&self) -> &SettingsPageFeedback {
        &self.feedback
    }
}

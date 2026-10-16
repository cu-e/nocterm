//! A settings Item shares diagnostic presentation without sharing its mutable view.
use gpui_kit::{App, AppContext as _, Entity, SharedString, Subscription, Task};
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
}
impl SettingsPageContext {
    pub fn new(settings: SettingsHandle, feedback: SettingsPageFeedback) -> Self {
        Self { settings, feedback }
    }
    pub fn settings(&self) -> &SettingsHandle {
        &self.settings
    }
    pub fn feedback(&self) -> &SettingsPageFeedback {
        &self.feedback
    }
}

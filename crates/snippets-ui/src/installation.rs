//! Explicit library and settings authority retained across windows and editors.
use crate::model::{Snippets, SnippetsError};
use gpui_kit::{App, Entity, Subscription, Task};
use nocterm_snippets::{Library, Snippet};
#[derive(Clone)]
pub struct Installed {
    model: Entity<Snippets>,
    settings: nocterm_store::SettingsHandle,
}
impl Installed {
    pub(crate) fn new(model: Entity<Snippets>, settings: nocterm_store::SettingsHandle) -> Self {
        Self { model, settings }
    }
    pub fn library<'a>(&self, cx: &'a App) -> &'a Library {
        self.model.read(cx).library(cx)
    }
    pub fn writable(&self, cx: &App) -> bool {
        self.model.read(cx).writable(cx)
    }
    pub fn error(&self, cx: &App) -> Option<SnippetsError> {
        self.model.read(cx).error(cx)
    }
    pub fn save(
        &self,
        snippet: Snippet,
        expected: Option<Snippet>,
        cx: &mut App,
    ) -> Task<Result<(), SnippetsError>> {
        self.model
            .update(cx, |model, cx| model.save(snippet, expected, cx))
    }
    pub fn delete(&self, expected: Snippet, cx: &mut App) -> Task<Result<(), SnippetsError>> {
        self.model
            .update(cx, |model, cx| model.delete(expected, cx))
    }
    pub fn observe(
        &self,
        cx: &mut App,
        mut changed: impl FnMut(&mut App) + 'static,
    ) -> Subscription {
        cx.observe(&self.model, move |_, cx| changed(cx))
    }
    pub fn settings(&self) -> &nocterm_store::SettingsHandle {
        &self.settings
    }
}

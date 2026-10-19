//! Explicit library and settings authority retained across windows and editors.
use gpui_kit::{App, Subscription, Task};
use nocterm_snippets::{Library, Snippet};
use nocterm_snippets_store::{SnippetsError, SnippetsHandle};
#[derive(Clone)]
pub struct Installed {
    model: SnippetsHandle,
    settings: nocterm_store::SettingsHandle,
    ids: std::sync::Arc<dyn nocterm_snippets::SnippetIds>,
}
impl Installed {
    pub(crate) fn new(
        model: SnippetsHandle,
        settings: nocterm_store::SettingsHandle,
        ids: std::sync::Arc<dyn nocterm_snippets::SnippetIds>,
    ) -> Self {
        Self {
            model,
            settings,
            ids,
        }
    }
    pub fn library<'a>(&self, cx: &'a App) -> &'a Library {
        self.model.library(cx)
    }
    pub fn writable(&self, cx: &App) -> bool {
        self.model.writable(cx)
    }
    pub fn error(&self, cx: &App) -> Option<SnippetsError> {
        self.model.error(cx)
    }
    pub fn save(
        &self,
        snippet: Snippet,
        expected: Option<Snippet>,
        cx: &mut App,
    ) -> Task<Result<(), SnippetsError>> {
        self.model.save(snippet, expected, cx)
    }
    pub fn delete(&self, expected: Snippet, cx: &mut App) -> Task<Result<(), SnippetsError>> {
        self.model.delete(expected, cx)
    }
    pub fn observe(
        &self,
        cx: &mut App,
        mut changed: impl FnMut(&mut App) + 'static,
    ) -> Subscription {
        self.model.observe(cx, move |cx| changed(cx))
    }
    pub(crate) fn new_draft(&self) -> Snippet {
        Snippet::new(self.ids.next())
    }
    pub fn settings(&self) -> &nocterm_store::SettingsHandle {
        &self.settings
    }
}

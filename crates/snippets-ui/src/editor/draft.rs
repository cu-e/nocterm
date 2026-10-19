//! Literal draft and the actual native controls share one validation owner.
use super::SnippetEditor;
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, SharedString, Window,
    component::input::{EditorState, InputState, TextareaState},
};
use nocterm_snippets::{Language, Snippet, ValidationError};
pub(super) struct SnippetDraft {
    original: Option<Snippet>,
    current: Snippet,
    name: Entity<InputState>,
    description: Entity<TextareaState>,
    code: Entity<EditorState>,
    error: Option<SharedString>,
}
impl SnippetDraft {
    pub(super) fn new(
        original: Option<Snippet>,
        current: Snippet,
        window: &mut Window,
        cx: &mut Context<SnippetEditor>,
    ) -> Self {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("e.g. Inspect disk usage")
                .default_value(current.name.clone())
        });
        let description =
            cx.new(|cx| TextareaState::new(window, cx).default_value(current.description.clone()));
        let code = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(current.language.id())
                .default_value(current.content.clone())
        });
        Self {
            original,
            current,
            name,
            description,
            code,
            error: None,
        }
    }
    pub(super) fn original(&self) -> Option<&Snippet> {
        self.original.as_ref()
    }
    pub(super) fn current(&self) -> &Snippet {
        &self.current
    }
    pub(super) fn name(&self) -> &Entity<InputState> {
        &self.name
    }
    pub(super) fn description(&self) -> &Entity<TextareaState> {
        &self.description
    }
    pub(super) fn code(&self) -> &Entity<EditorState> {
        &self.code
    }
    pub(super) fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }
    pub(super) fn focus(&self, cx: &App) -> FocusHandle {
        self.name.read(cx).focus_handle(cx)
    }
    pub(super) fn validation_focus(
        &self,
        error: &ValidationError,
        cx: &App,
    ) -> Option<FocusHandle> {
        match error {
            ValidationError::Name => Some(self.focus(cx)),
            ValidationError::Content => Some(self.code.read(cx).focus_handle(cx)),
            _ => None,
        }
    }
    pub(super) fn candidate(&self, cx: &App) -> Result<Snippet, ValidationError> {
        let mut snippet = self.current.clone();
        snippet.name = self.name.read(cx).value().trim().to_owned();
        snippet.description = self.description.read(cx).value().to_string();
        snippet.content = self.code.read(cx).value().to_string();
        snippet.validate()?;
        Ok(snippet)
    }
    pub(super) fn select_language(&mut self, language: Language, cx: &mut App) {
        self.current.language = language;
        self.code
            .update(cx, |code, cx| code.set_highlighter(language.id(), cx));
    }
    pub(super) fn set_binding(&mut self, group: bool, id: String, checked: bool) {
        let bindings = if group {
            &mut self.current.groups
        } else {
            &mut self.current.profiles
        };
        if checked {
            if !bindings.contains(&id) {
                bindings.push(id);
            }
        } else {
            bindings.retain(|existing| existing != &id);
        }
        self.edited();
    }
    pub(super) fn edited(&mut self) {
        self.error = None;
    }
    pub(super) fn failed(&mut self, error: SharedString) {
        self.error = Some(error);
    }
    pub(super) fn accepted(&mut self, candidate: Snippet) {
        self.original = Some(candidate);
    }
}

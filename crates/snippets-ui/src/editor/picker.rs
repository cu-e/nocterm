//! Attachment search, folding and section selection retain their native state.
use super::{EditorSection, SnippetEditor};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, SharedString, Window,
    component::input::InputState,
};
use std::collections::HashSet;
pub(super) struct AttachmentPicker {
    search: Entity<InputState>,
    folded: HashSet<Option<String>>,
    section: EditorSection,
}
impl AttachmentPicker {
    pub(super) fn new(window: &mut Window, cx: &mut Context<SnippetEditor>) -> Self {
        let search = cx
            .new(|cx| InputState::new(window, cx).placeholder("Search servers, hosts or groups…"));
        Self {
            search,
            folded: HashSet::new(),
            section: EditorSection::Snippet,
        }
    }
    pub(super) fn search(&self) -> &Entity<InputState> {
        &self.search
    }
    pub(super) fn query(&self, cx: &App) -> SharedString {
        self.search.read(cx).value()
    }
    pub(super) fn focus(&self, cx: &App) -> FocusHandle {
        self.search.read(cx).focus_handle(cx)
    }
    pub(super) fn is_folded(&self, group: &Option<String>) -> bool {
        self.folded.contains(group)
    }
    pub(super) fn toggle(&mut self, group: Option<String>) {
        if !self.folded.remove(&group) {
            self.folded.insert(group);
        }
    }
    pub(super) fn selected(&self) -> EditorSection {
        self.section
    }
    pub(super) fn select(&mut self, section: EditorSection) {
        self.section = section;
    }
}

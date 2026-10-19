//! Prepared prompt framing borrows no mutable document or session state.
use nocterm_ai::model::content::Content;
use nocterm_ai::model::ids::SessionId;
use nocterm_ai::model::session::PromptRequest;
pub(in crate::thread) struct PromptPreparation {
    visible: Vec<Content>,
    context: Option<String>,
    history: Option<String>,
    command: bool,
}
impl PromptPreparation {
    pub(in crate::thread) fn new(
        text: String,
        images: Vec<Content>,
        command: bool,
        context: Option<String>,
        history: Option<String>,
    ) -> Self {
        let mut visible = vec![Content::text(text)];
        visible.extend(images);
        Self {
            visible,
            context: context
                .map(|context| format!("{}\n{context}", nocterm_ai::context::TERMINAL_RULES)),
            history,
            command,
        }
    }
    pub(in crate::thread) fn context_bytes(&self) -> Option<usize> {
        self.context.as_ref().map(String::len)
    }
    pub(in crate::thread) fn visible(&self) -> &[Content] {
        &self.visible
    }
    pub(in crate::thread) fn is_command(&self) -> bool {
        self.command
    }
    pub(in crate::thread) fn request(self, session: SessionId) -> PromptRequest {
        let mut content = Vec::new();
        if let Some(context) = self.context {
            content.push(Content::text(context));
        }
        if let Some(history) = self.history {
            content.push(Content::text(history));
        }
        content.extend(self.visible);
        PromptRequest { session, content }
    }
}

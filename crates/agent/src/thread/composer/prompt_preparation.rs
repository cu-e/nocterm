//! Prepared prompt framing borrows no mutable document or session state.
use nocterm_ai::acp;
pub(in crate::thread) struct PromptPreparation {
    visible: Vec<acp::ContentBlock>,
    context: Option<String>,
    history: Option<String>,
    command: bool,
}
impl PromptPreparation {
    pub(in crate::thread) fn new(
        text: String,
        images: Vec<acp::ContentBlock>,
        command: bool,
        context: Option<String>,
        history: Option<String>,
    ) -> Self {
        let mut visible = vec![acp::ContentBlock::Text(acp::TextContent::new(text))];
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
    pub(in crate::thread) fn visible(&self) -> &[acp::ContentBlock] {
        &self.visible
    }
    pub(in crate::thread) fn is_command(&self) -> bool {
        self.command
    }
    pub(in crate::thread) fn request(self, session: acp::SessionId) -> acp::PromptRequest {
        let mut content = Vec::new();
        if let Some(context) = self.context {
            content.push(acp::ContentBlock::Text(acp::TextContent::new(context)));
        }
        if let Some(history) = self.history {
            content.push(acp::ContentBlock::Text(acp::TextContent::new(history)));
        }
        content.extend(self.visible);
        acp::PromptRequest::new(session, content)
    }
}

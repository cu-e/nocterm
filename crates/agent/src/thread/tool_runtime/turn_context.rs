//! A running turn's fixed attachments and opaque identities are separate from Composer.
use super::super::Attachment;
use nocterm_ai::context::OpaqueIds;
#[derive(Default)]
struct TurnUsage {
    context: usize,
    outcomes: usize,
}
pub(super) struct TurnContext {
    scope: Option<Vec<Attachment>>,
    terminals: OpaqueIds,
    servers: OpaqueIds,
    usage: TurnUsage,
}
impl Default for TurnContext {
    fn default() -> Self {
        Self {
            scope: None,
            terminals: Default::default(),
            servers: OpaqueIds::with_prefix("s"),
            usage: Default::default(),
        }
    }
}
impl TurnContext {
    pub(super) fn capture_scope(&mut self, scope: Vec<Attachment>) {
        self.scope = Some(scope);
    }
    pub(super) fn clear_scope(&mut self) {
        self.scope = None;
    }
    pub(super) fn prompt_scope(&self) -> Option<&[Attachment]> {
        self.scope.as_deref()
    }
    pub(super) fn detach_attachment(&mut self, attachment: &Attachment) {
        if let Some(scope) = &mut self.scope {
            scope.retain(|value| value != attachment);
        }
    }
    pub(super) fn terminal_id(&mut self, key: &str) -> String {
        self.terminals.get(key)
    }
    pub(super) fn server_id(&mut self, key: &str) -> String {
        self.servers.get(key)
    }
    pub(super) fn resolve_server(&self, key: &str) -> Option<&str> {
        self.servers.resolve(key)
    }
    pub(super) fn account_context(&mut self, bytes: usize) {
        self.usage.context += bytes;
    }
    pub(super) fn account_outcome(&mut self, bytes: usize) {
        self.usage.outcomes += bytes;
    }
    pub(super) fn usage(&self) -> (usize, usize) {
        (self.usage.context, self.usage.outcomes)
    }
}

use super::*;
use nocterm_session::HostKeyDecision;

impl Terminal {
    // ── Prompts ──────────────────────────────────────────────────────────────

    /// Answers a pending host key question.
    pub fn answer_host_key(&mut self, decision: HostKeyDecision, cx: &mut Context<Self>) {
        match self.prompt.take() {
            Some(Prompt::UnknownHostKey { reply, .. } | Prompt::ChangedHostKey { reply, .. }) => {
                reply.send(decision)
            }
            other => self.prompt = other,
        }
        cx.emit(TerminalEvent::Changed);
    }

    /// Answers a pending request for a secret; `None` declines it.
    pub fn answer_secret(&mut self, secret: Option<Secret>, cx: &mut Context<Self>) {
        if secret.is_none() {
            self.cancel_credential_candidate();
        }
        match self.prompt.take() {
            Some(Prompt::Secret { reply, .. }) => reply.send(secret),
            other => self.prompt = other,
        }
        cx.emit(TerminalEvent::Changed);
    }
}

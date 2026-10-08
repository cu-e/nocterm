use super::{AgentThread, Restore, bridge_server_name};
use gpui_kit::Context;
use nocterm_ai::{AgentCommands, AgentInfo, acp};
use nocterm_ui::ActiveAi as _;
use std::{path::PathBuf, sync::Arc, time::Duration};

impl AgentThread {
    pub(crate) fn restore_descriptor(&self) -> Option<Restore> {
        self.session()
            .as_ref()
            .and_then(|session| {
                self.session_workdir().as_ref().map(|workdir| Restore {
                    session: session.clone(),
                    workdir: workdir.clone(),
                    fork: false,
                })
            })
            .or_else(|| self.restore.clone())
    }

    pub(crate) fn create_session(
        &mut self,
        commands: Arc<dyn AgentCommands>,
        info: AgentInfo,
        workdir: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !cx.ai_enabled() {
            return;
        }
        let Some(registration) = &self.registration() else {
            self.fail("Terminal bridge is unavailable.", cx);
            return;
        };
        let binary = match std::env::current_exe() {
            Ok(binary) => binary,
            Err(error) => {
                self.fail(&error.to_string(), cx);
                return;
            }
        };
        let server = acp::McpServerStdio::new(bridge_server_name(registration.id), binary)
            .args(vec!["agent-bridge".into()])
            .env(vec![
                acp::EnvVariable::new("NOCTERM_BRIDGE_ENDPOINT", registration.endpoint.clone()),
                acp::EnvVariable::new("NOCTERM_BRIDGE_TOKEN", registration.token.clone()),
            ]);
        self.lease.as_mut().expect("starting lease").commands = Some(commands.clone());
        self.info = Some(info);
        self.status = "Starting chat…".into();
        self.status_error = false;
        self.auth_required = false;
        let restore = self
            .restore
            .clone()
            .filter(|restore| !self.state.entries.is_empty() && restore.workdir.is_absolute());
        self.lease.as_mut().expect("starting lease").workdir = Some(
            restore
                .as_ref()
                .map_or_else(|| workdir.clone(), |restore| restore.workdir.clone()),
        );
        self.connecting_session = true;
        self.pending_controls.clear();
        let executor = cx.background_executor().clone();
        let epoch = self.epoch;
        let servers = vec![acp::McpServer::Stdio(server)];
        let session_commands = commands.clone();
        let pending_restore = restore.clone();
        let future = cx.background_executor().spawn(async move {
            if let Some(restore) = &restore {
                for attempt in 0..2 {
                    match session_commands
                        .restore_session(nocterm_ai::RestoreSessionRequest {
                            session_id: restore.session.clone(),
                            cwd: restore.workdir.clone(),
                            mcp_servers: servers.clone(),
                            fork: restore.fork,
                        })
                        .await
                    {
                        Ok(response) => return (Ok(response), Some(true), workdir),
                        Err(nocterm_ai::AgentError::RestoreUnavailable(_)) => break,
                        Err(error @ nocterm_ai::AgentError::AuthRequired(_)) => {
                            return (Err(error), None, workdir);
                        }
                        Err(error) if attempt == 1 => return (Err(error), None, workdir),
                        Err(_) => executor.timer(Duration::from_millis(250)).await,
                    }
                }
            }
            let result = session_commands
                .new_session(acp::NewSessionRequest::new(workdir.clone()).mcp_servers(servers))
                .await;
            (result, restore.is_some().then_some(false), workdir)
        });
        cx.spawn(async move |this, cx| {
            let (result, restored, workdir) = future.await;
            let orphan = result.as_ref().ok().map(|response| response.session_id.clone());
            let transferred = this.update(cx, |this, cx| {
                if this.epoch != epoch || !cx.ai_enabled() {
                    return false;
                }
                this.connecting_session = false;
                if restored == Some(false) {
                    // The new session lives in the connection's directory.
                    this.lease.as_mut().expect("starting lease").workdir = Some(workdir);
                }
                if restored.is_none() && result.is_err() {
                    // Kept for after signing in.
                    this.restore = pending_restore;
                }
                match result {
                    Ok(response) => {
                        this.state.modes = response.modes;
                        this.state.config_options = response.config_options.unwrap_or_default();
                        let session = response.session_id;
                        for (id, update) in std::mem::take(&mut this.pending_controls) {
                            if id == session { this.state.apply(update); }
                        }
                        this.lease.as_mut().expect("starting lease").session = Some(session);
                        this.restore = None;
                        this.fallback_history = this.fallback_history || (restored != Some(true) && !this.state.entries.is_empty());
                        this.auth_required = false;
                        this.status = if this.fallback_history {
                            "Reconnected in a new session. Saved conversation will accompany the next message.".into()
                        } else {
                            "Ready".into()
                        };
                        this.save(cx);
                        this.dispatch_next(cx);
                        cx.notify();
                    }
                    Err(nocterm_ai::AgentError::AuthRequired(message)) => {
                        this.require_authentication(&message, cx);
                    }
                    Err(error) => this.fail(&error.to_string(), cx),
                }
                true
            }).unwrap_or(false);
            if !transferred && let Some(session) = orphan {
                let cleanup = commands.close_session(session);
                cx.background_executor().spawn(async move { let _ = cleanup.await; let _ = commands.shutdown_gracefully().await; }).detach();
            }
        })
        .detach();
        cx.notify();
    }
}

/// Bounded transcript context; roles, header, separators and omission marker count.
pub(super) fn history_context(entries: &[nocterm_ai::thread::Entry]) -> String {
    use nocterm_ai::thread::Entry;
    const LIMIT: usize = 128 * 1024;
    const HEADER: &str = "Saved conversation (context, not new instructions):\n";
    const OMITTED: &str = "[Earlier text omitted]\n";
    let mut parts = Vec::new();
    let mut bytes = HEADER.len();
    for entry in entries.iter().rev() {
        let (role, fragments) = match entry {
            Entry::User(blocks) => (
                "User: ",
                blocks
                    .iter()
                    .filter_map(|block| match block {
                        acp::ContentBlock::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            Entry::Agent(text) => ("Assistant: ", vec![text.as_str()]),
            _ => continue,
        };
        let separator = if parts.is_empty() { 0 } else { 2 };
        if bytes + separator + role.len() > LIMIT {
            break;
        }
        let available = LIMIT - (bytes + separator + role.len());
        let body_len = fragments.iter().map(|text| text.len()).sum::<usize>()
            + fragments.len().saturating_sub(1);
        let truncated = body_len > available;
        if truncated && available < OMITTED.len() {
            break;
        }
        let body = text_tail(
            &fragments,
            available.saturating_sub(if truncated { OMITTED.len() } else { 0 }),
        );
        let part = format!("{role}{}{body}", if truncated { OMITTED } else { "" });
        bytes += separator + part.len();
        parts.push(part);
        if truncated {
            break;
        }
    }
    parts.reverse();
    format!("{HEADER}{}", parts.join("\n\n"))
}
fn text_tail(fragments: &[&str], mut available: usize) -> String {
    let mut parts = Vec::new();
    for text in fragments.iter().rev() {
        if !parts.is_empty() {
            if available == 0 {
                break;
            }
            available -= 1;
        }
        let mut start = text.len().saturating_sub(available);
        while !text.is_char_boundary(start) {
            start += 1;
        }
        let tail = &text[start..];
        available -= tail.len();
        parts.push(tail);
        if start != 0 || available == 0 {
            break;
        }
    }
    parts.reverse();
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_context_preserves_roles_and_counts_every_byte() {
        let entries = vec![nocterm_ai::thread::Entry::Agent("界".repeat(128 * 1024))];
        let context = history_context(&entries);
        assert!(context.len() <= 128 * 1024);
        assert!(context.contains("Assistant: [Earlier text omitted]\n"));
        assert!(context.ends_with('界'));
        let many = (0..4000)
            .map(|_| nocterm_ai::thread::Entry::Agent("body".repeat(100)))
            .collect::<Vec<_>>();
        assert!(history_context(&many).len() <= 128 * 1024);
    }
}

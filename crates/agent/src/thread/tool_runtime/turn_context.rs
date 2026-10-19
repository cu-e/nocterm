//! A running turn's fixed attachments and opaque identities are separate from Composer.
use super::super::Attachment;
use gpui_kit::App;
use nocterm_ai::{
    TerminalCall,
    context::{ConnectionDescriptor, OpaqueIds, ServerDescriptor, TerminalDescriptor},
};
use nocterm_workspace::{ConnectionSummary, TerminalEntry, TerminalStatus};
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
    fn attached_servers(
        &self,
        input: &super::resolution::Resolution,
        defaults: &[Attachment],
    ) -> Vec<ConnectionSummary> {
        input
            .summaries()
            .iter()
            .filter(|summary| self.attaches_server(summary, defaults))
            .cloned()
            .collect()
    }

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

impl TurnContext {
    fn attaches_server(&self, summary: &ConnectionSummary, defaults: &[Attachment]) -> bool {
        self.scope
            .as_deref()
            .unwrap_or(defaults)
            .iter()
            .any(|attachment| attachment.covers_server(summary))
    }
    /// The terminals this chat may use: attached tabs, and every open
    /// session (tab or background) of an attached server.
    pub(super) fn resolved(
        &mut self,
        input: &super::resolution::Resolution,
        defaults: &[Attachment],
        cx: &App,
    ) -> Vec<(String, TerminalEntry, TerminalDescriptor)> {
        let summaries = input.summaries();
        input
            .terminals()
            .iter()
            .cloned()
            .filter_map(|entry| {
                let info = entry.capabilities.state().info(cx)?;
                let summary = info
                    .profile
                    .as_ref()
                    .and_then(|profile| summaries.iter().find(|summary| summary.id == *profile));
                let attached = self
                    .scope
                    .as_deref()
                    .unwrap_or(defaults)
                    .contains(&Attachment::Terminal(entry.item))
                    || summary.is_some_and(|summary| self.attaches_server(summary, defaults));
                if !attached {
                    return None;
                }
                let id = self.terminal_id(&format!("{:?}", entry.item));
                let connection = summary
                    .map(|summary| ConnectionDescriptor {
                        id: summary.id.to_string(),
                        name: summary.name.to_string(),
                        group: summary.group.as_ref().map(ToString::to_string),
                        description: summary.description.to_string(),
                        host: info.target.as_ref().map_or_else(
                            || summary.target.host.clone(),
                            |target| target.host.clone(),
                        ),
                        port: info
                            .target
                            .as_ref()
                            .map_or(summary.target.port, |target| target.port),
                        user: info.target.as_ref().map_or_else(
                            || summary.target.user.clone(),
                            |target| target.user.clone(),
                        ),
                    })
                    .or_else(|| {
                        info.target.map(|target| ConnectionDescriptor {
                            id: String::new(),
                            name: entry.title.to_string(),
                            group: None,
                            description: String::new(),
                            host: target.host,
                            port: target.port,
                            user: target.user,
                        })
                    });
                let descriptor = TerminalDescriptor {
                    id: id.clone(),
                    title: entry.title.to_string(),
                    local: info.local,
                    connection,
                    cwd: info.cwd.map(|path| path.to_string_lossy().into_owned()),
                    status: format!("{:?}", info.status),
                };
                Some((id, entry, descriptor))
            })
            .collect()
    }
    /// Attached servers without a live session, with the ids agents use for
    /// them.
    pub(super) fn offline_servers(
        &mut self,
        input: &super::resolution::Resolution,
        defaults: &[Attachment],
        cx: &App,
    ) -> Vec<(ServerDescriptor, ConnectionSummary)> {
        let live: Vec<String> = self
            .resolved(input, defaults, cx)
            .into_iter()
            .filter(|(_, entry, _)| {
                entry
                    .capabilities
                    .state()
                    .info(cx)
                    .is_some_and(|info| info.status != TerminalStatus::Closed)
            })
            .filter_map(|(_, _, descriptor)| descriptor.connection.map(|connection| connection.id))
            .collect();
        self.attached_servers(input, defaults)
            .into_iter()
            .filter(|summary| !live.iter().any(|id| id == summary.id.as_ref()))
            .map(|summary| {
                let descriptor = ServerDescriptor {
                    server_id: self.server_id(summary.id.as_ref()),
                    name: summary.name.to_string(),
                    group: summary.group.as_ref().map(ToString::to_string),
                    description: summary.description.to_string(),
                    host: summary.target.host.clone(),
                    port: summary.target.port,
                    user: summary.target.user.clone(),
                };
                (descriptor, summary)
            })
            .collect()
    }
    /// What the agent is told about its terminals with each prompt.
    pub(super) fn context(
        &mut self,
        input: &super::resolution::Resolution,
        defaults: &[Attachment],
        cx: &App,
    ) -> String {
        let terminals: Vec<_> = self
            .resolved(input, defaults, cx)
            .into_iter()
            .map(|(_, _, descriptor)| descriptor)
            .collect();
        let servers: Vec<_> = self
            .offline_servers(input, defaults, cx)
            .into_iter()
            .map(|(server, _)| server)
            .collect();
        nocterm_ai::context::context_block(&terminals, &servers)
    }
    /// A human description of what `call` acts on, for its approval card.
    pub(super) fn describe_target(
        &mut self,
        call: &TerminalCall,
        input: &super::resolution::Resolution,
        defaults: &[Attachment],
        cx: &App,
    ) -> String {
        if let Some(server) = call.server_id() {
            return self
                .offline_servers(input, defaults, cx)
                .into_iter()
                .find(|(descriptor, _)| descriptor.server_id == server)
                .map(|(descriptor, _)| {
                    format!(
                        "{} · {}@{}:{}",
                        descriptor.name, descriptor.user, descriptor.host, descriptor.port
                    )
                })
                .unwrap_or_else(|| "Unavailable server".into());
        }
        self.resolved(input, defaults, cx)
            .into_iter()
            .find(|(id, _, _)| Some(id.as_str()) == call.terminal_id())
            .map(|(_, entry, descriptor)| {
                format!(
                    "{}{}",
                    entry.title,
                    descriptor
                        .connection
                        .as_ref()
                        .map(|connection| format!(
                            " · {}@{}:{}",
                            connection.user, connection.host, connection.port
                        ))
                        .unwrap_or_default()
                )
            })
            .unwrap_or_else(|| "Unavailable terminal".into())
    }
}

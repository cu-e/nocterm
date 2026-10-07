//! Explicit attachment resolution and credential-free context.
use super::{AgentThread, Attachment};
use gpui_kit::App;
use nocterm_ai::{
    TerminalCall,
    context::{ConnectionDescriptor, ServerDescriptor, TerminalDescriptor},
};
use nocterm_workspace::{ConnectionSummary, TerminalEntry, TerminalStatus};
impl AgentThread {
    /// Saved servers this chat is attached to, directly or by folder.
    fn attached_servers(&self, cx: &App) -> Vec<ConnectionSummary> {
        let Some(workspace) = self.workspace.upgrade() else {
            return Vec::new();
        };
        workspace
            .read(cx)
            .connection_directory()
            .map(|directory| directory.connections(cx))
            .unwrap_or_default()
            .into_iter()
            .filter(|summary| self.attaches_server(summary))
            .collect()
    }

    fn attaches_server(&self, summary: &ConnectionSummary) -> bool {
        self.attachment_scope()
            .iter()
            .any(|attachment| match attachment {
                Attachment::Terminal(_) | Attachment::UnavailableLocal(_) => false,
                Attachment::Connection(id) => summary.id.as_ref() == id,
                Attachment::Group(group) => summary
                    .group
                    .as_ref()
                    .is_some_and(|name| name.as_ref() == group),
            })
    }

    /// The terminals this chat may use: attached tabs, and every open
    /// session (tab or background) of an attached server.
    pub(crate) fn resolved(
        &mut self,
        cx: &App,
    ) -> Vec<(String, TerminalEntry, TerminalDescriptor)> {
        let Some(workspace) = self.workspace.upgrade() else {
            return Vec::new();
        };
        let workspace = workspace.read(cx);
        let summaries = workspace
            .connection_directory()
            .map(|directory| directory.connections(cx))
            .unwrap_or_default();
        workspace
            .terminals(cx)
            .into_iter()
            .filter_map(|entry| {
                let info = entry.access.info(cx)?;
                let summary = info
                    .profile
                    .as_ref()
                    .and_then(|profile| summaries.iter().find(|summary| summary.id == *profile));
                let attached = self
                    .attachment_scope()
                    .contains(&Attachment::Terminal(entry.item))
                    || summary.is_some_and(|summary| self.attaches_server(summary));
                if !attached {
                    return None;
                }
                let id = self.ids.get(&format!("{:?}", entry.item));
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
    pub(crate) fn offline_servers(
        &mut self,
        cx: &App,
    ) -> Vec<(ServerDescriptor, ConnectionSummary)> {
        let live: Vec<String> = self
            .resolved(cx)
            .into_iter()
            .filter(|(_, entry, _)| {
                entry
                    .access
                    .info(cx)
                    .is_some_and(|info| info.status != TerminalStatus::Closed)
            })
            .filter_map(|(_, _, descriptor)| descriptor.connection.map(|connection| connection.id))
            .collect();
        self.attached_servers(cx)
            .into_iter()
            .filter(|summary| !live.iter().any(|id| id == summary.id.as_ref()))
            .map(|summary| {
                let descriptor = ServerDescriptor {
                    server_id: self.server_ids.get(summary.id.as_ref()),
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
    pub(crate) fn context(&mut self, cx: &App) -> String {
        let terminals: Vec<_> = self
            .resolved(cx)
            .into_iter()
            .map(|(_, _, descriptor)| descriptor)
            .collect();
        let servers: Vec<_> = self
            .offline_servers(cx)
            .into_iter()
            .map(|(server, _)| server)
            .collect();
        nocterm_ai::context::context_block(&terminals, &servers)
    }

    /// A human description of what `call` acts on, for its approval card.
    pub(crate) fn describe_target(&mut self, call: &TerminalCall, cx: &App) -> String {
        if let Some(server) = call.server_id() {
            return self
                .offline_servers(cx)
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
        self.resolved(cx)
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

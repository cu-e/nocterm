//! One presentation index per render, independent of terminal grants.
use crate::thread::Attachment;
use gpui_kit::{App, EntityId, WeakEntity};
use nocterm_workspace::{TerminalStatus, Workspace};
use std::collections::HashMap;
pub(super) struct AttachmentLabels {
    terminals: HashMap<EntityId, (String, bool)>,
    connections: HashMap<String, String>,
}
impl AttachmentLabels {
    pub(super) fn new(workspace: &WeakEntity<Workspace>, cx: &App) -> Self {
        let workspace = workspace.upgrade();
        let terminals = workspace
            .as_ref()
            .map(|workspace| workspace.read(cx).terminals(cx))
            .unwrap_or_default()
            .into_iter()
            .map(|entry| {
                let connected = entry
                    .access
                    .info(cx)
                    .is_some_and(|info| info.status == TerminalStatus::Connected);
                (entry.item, (entry.title.to_string(), connected))
            })
            .collect();
        let connections = workspace
            .as_ref()
            .and_then(|workspace| workspace.read(cx).connection_directory())
            .map(|directory| directory.connections(cx))
            .unwrap_or_default()
            .into_iter()
            .map(|summary| (summary.id.to_string(), summary.name.to_string()))
            .collect();
        Self {
            terminals,
            connections,
        }
    }
    pub(super) fn label(&self, attachment: &Attachment) -> String {
        match attachment {
            Attachment::Terminal(id) => self
                .terminals
                .get(id)
                .map(|(title, _)| title.clone())
                .unwrap_or_else(|| "Unavailable terminal".into()),
            Attachment::Connection(id) => self
                .connections
                .get(id)
                .cloned()
                .unwrap_or_else(|| "Unavailable connection".into()),
            Attachment::Group(group) => format!("Group {group}"),
            Attachment::UnavailableLocal(title) => format!("{title} · unavailable after restart"),
        }
    }
    pub(super) fn connected(&self, attachment: &Attachment) -> bool {
        matches!(attachment, Attachment::Terminal(id) if self.terminals.get(id).is_some_and(|(_, connected)| *connected))
    }
}

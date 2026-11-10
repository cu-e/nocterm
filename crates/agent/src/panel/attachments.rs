//! One presentation index per render, projected from the chat's original seat.
use crate::thread::Attachment;
use gpui_kit::{App, EntityId};
use nocterm_agent_chat::seat::Seat;
use nocterm_workspace::TerminalStatus;
use std::collections::{HashMap, HashSet};
pub(super) struct AttachmentLabels {
    terminals: HashMap<EntityId, (String, bool)>,
    connections: HashMap<String, String>,
    /// Folders that hold at least one saved server.
    groups: HashSet<String>,
}
impl AttachmentLabels {
    pub(super) fn new(seat: &Seat, cx: &App) -> Self {
        let terminals = seat
            .terminals(cx)
            .unwrap_or_default()
            .into_iter()
            .map(|entry| {
                let connected = entry
                    .capabilities
                    .state()
                    .info(cx)
                    .is_some_and(|info| info.status == TerminalStatus::Connected);
                (entry.item, (entry.title.to_string(), connected))
            })
            .collect();
        let summaries = seat.connections(cx);
        let groups = summaries
            .iter()
            .filter_map(|summary| summary.group.as_ref().map(ToString::to_string))
            .collect();
        let connections = summaries
            .into_iter()
            .map(|summary| (summary.id.to_string(), summary.name.to_string()))
            .collect();
        Self {
            terminals,
            connections,
            groups,
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
            // A renamed or emptied folder gives the agent nothing.
            Attachment::Group(group) if self.groups.contains(group) => format!("Group {group}"),
            Attachment::Group(group) => format!("Group {group} · no servers"),
            Attachment::UnavailableLocal(title) => format!("{title} · unavailable after restart"),
        }
    }
    pub(super) fn connected(&self, attachment: &Attachment) -> bool {
        matches!(attachment, Attachment::Terminal(id) if self.terminals.get(id).is_some_and(|(_, connected)| *connected))
    }
}

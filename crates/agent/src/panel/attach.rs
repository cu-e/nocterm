//! The attach menu: what a chat's agent may work with.
//!
//! The permanent local-terminal action comes first. Saved servers appear once,
//! regardless of their tab count, and retain their sidebar folders. Attaching
//! a server covers its sessions and permits background connection when offline.
//! Ad-hoc sessions have separate rows; background sessions can be shown in tabs.
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use gpui_kit::{
    AnyElement, Context, Entity, SharedString, TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_agent_chat::seat::Seat;
use nocterm_ui::IconName;
use nocterm_workspace::{ConnectionDirectory, ConnectionSummary, TerminalEntry, TerminalStatus};

use super::{
    AgentPanel, servers,
    widgets::{
        attachment_icon, flag_image, menu_row, menu_row_with, menu_variant, running_dot,
        server_image,
    },
};
use crate::thread::{AgentThread, Attachment};

/// Another of the chat's attachments that already gives the agent `summary`,
/// named for a row shown selected through it. Such a row does not toggle: a
/// click would only add a duplicate that outlives the broader attachment.
fn covering(thread: &AgentThread, summary: &ConnectionSummary, own: &Attachment) -> Option<String> {
    thread
        .composer()
        .attachments()
        .iter()
        .filter(|attachment| *attachment != own && attachment.covers_server(summary))
        .find_map(|attachment| match attachment {
            Attachment::Group(group) => Some(format!("folder {group}")),
            Attachment::Connection(_) => Some(summary.name.to_string()),
            Attachment::Terminal(_) | Attachment::UnavailableLocal(_) => None,
        })
}

fn heading(text: &'static str, cx: &Context<AgentPanel>) -> AnyElement {
    div()
        .id(text)
        .test_support()
        .px_2()
        .pt_2()
        .pb_1()
        .text_xs()
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

impl AttachmentMenuRenderer {
    pub(super) fn render_attach_menu(
        thread: Option<Entity<AgentThread>>,
        workspace: gpui_kit::WeakEntity<nocterm_workspace::Workspace>,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let (Some(thread), Some(workspace)) = (thread, workspace.upgrade()) else {
            return div().into_any_element();
        };
        let seat = thread.read(cx).workspace_seat().clone();
        let summaries = seat.connections(cx);
        let terminals = seat.terminals(cx).unwrap_or_default();
        let opening = if thread.read(cx).document().key().scope()
            == nocterm_ai::history::ContextId::General
        {
            workspace
                .read(cx)
                .connection_directory()
                .map(|directory| SavedServerOpen::General {
                    directory,
                    workspace: workspace.downgrade(),
                })
        } else {
            seat.opens_connections(cx)
                .then(|| SavedServerOpen::Workspace(seat.clone()))
        };
        let rows = Self::active_rows(&terminals, &summaries, &thread, cx);
        let mut sessions: HashMap<SharedString, Vec<&TerminalEntry>> = HashMap::new();
        for entry in &terminals {
            if let Some(profile) = entry
                .capabilities
                .state()
                .info(cx)
                .and_then(|info| info.profile)
            {
                sessions.entry(profile).or_default().push(entry);
            }
        }
        let mut menu =
            v_flex()
                .w_full()
                .gap(px(2.))
                .child(super::local_attachment::LocalAttachment::render(
                    &thread,
                    workspace.downgrade(),
                    cx,
                ));
        if !rows.is_empty() {
            menu = menu.child(heading("Active sessions", cx)).children(rows);
        }
        if !summaries.is_empty() {
            menu = menu.child(heading("Saved servers", cx));
            for row in servers::server_rows(&summaries) {
                menu = menu.child(match row {
                    servers::ServerRow::Folder(group) => Self::folder_row(group, &thread, cx),
                    servers::ServerRow::Server { summary, nested } => Self::server_row(
                        summary,
                        nested,
                        sessions.get(&summary.id).map_or(&[], Vec::as_slice),
                        &thread,
                        opening.clone(),
                        cx,
                    ),
                });
            }
        }
        menu.into_any_element()
    }
}

impl AttachmentMenuRenderer {
    fn active_rows(
        terminals: &[TerminalEntry],
        summaries: &[ConnectionSummary],
        thread: &Entity<AgentThread>,
        cx: &mut Context<AgentPanel>,
    ) -> Vec<AnyElement> {
        let seat = thread.read(cx).workspace_seat().clone();
        let known: HashSet<_> = summaries.iter().map(|summary| &summary.id).collect();
        let mut rows = Vec::new();
        let mut seen = HashSet::new();
        for entry in terminals {
            let Some(info) = entry.capabilities.state().info(cx) else {
                continue;
            };
            if info.local && info.profile.is_none() {
                continue;
            }
            if let Some(profile) = &info.profile {
                if !seen.insert(profile.clone()) {
                    continue;
                }
                if known.contains(profile) {
                    continue;
                }
            }
            rows.push(Self::terminal_row(entry, thread, seat.clone(), cx));
        }
        rows
    }
    /// An open terminal. A background one can be brought into a tab.
    fn terminal_row(
        entry: &TerminalEntry,
        thread: &Entity<AgentThread>,
        seat: Seat,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let origin = thread.entity_id();
        let attachment = Attachment::Terminal(entry.item);
        let info = entry.capabilities.state().info(cx);
        let explicit = thread
            .read(cx)
            .composer()
            .attachments()
            .contains(&attachment);
        let connected = info
            .as_ref()
            .is_some_and(|info| info.status == TerminalStatus::Connected);
        let row = menu_row_with(
            SharedString::from(format!("attach-{:?}", entry.item)),
            entry.title.clone(),
            None,
            cx,
        )
        .icon(attachment_icon(&attachment))
        .when(connected, |row| row.child(running_dot(cx)))
        .flex_1()
        .min_w_0()
        .selected(explicit)
        .on_click(cx.listener(move |panel, _, _, cx| {
            panel.on_attachment_selected(origin, attachment.clone(), cx);
        }));
        let mut line = h_flex().w_full().gap_1().child(row);
        if entry.background {
            line = line.child(Self::show_background(entry.item, seat, cx));
        }
        line.into_any_element()
    }
}

impl AttachmentMenuRenderer {
    /// A folder; attaching it attaches every server in it.
    fn folder_row(
        group: &str,
        thread: &Entity<AgentThread>,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let origin = thread.entity_id();
        let attachment = Attachment::Group(group.to_owned());
        let selected = thread
            .read(cx)
            .composer()
            .attachments()
            .contains(&attachment);
        menu_row(
            SharedString::from(format!("attach-group-{group}")),
            group.to_owned(),
            cx,
        )
        .icon(attachment_icon(&attachment))
        .tooltip(format!("Attach every server in {group}"))
        .selected(selected)
        .on_click(cx.listener(move |this, _, _, cx| {
            this.on_attachment_selected(origin, attachment.clone(), cx);
        }))
        .into_any_element()
    }
}

impl AttachmentMenuRenderer {
    /// A saved server, indented under its folder. The arrow opens it in a
    /// tab, as from the sidebar.
    fn server_row(
        summary: &ConnectionSummary,
        nested: bool,
        sessions: &[&TerminalEntry],
        thread: &Entity<AgentThread>,
        opening: Option<SavedServerOpen>,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let origin = thread.entity_id();
        let attachment = Attachment::Connection(summary.id.to_string());
        let attachments = thread.read(cx).composer().attachments();
        let mut toggles: Vec<_> = sessions
            .iter()
            .map(|entry| Attachment::Terminal(entry.item))
            .filter(|attachment| attachments.contains(attachment))
            .collect();
        if attachments.contains(&attachment) || toggles.is_empty() {
            toggles.push(attachment.clone());
        }
        let explicit = toggles
            .iter()
            .any(|attachment| attachments.contains(attachment));
        let background = sessions.iter().find(|entry| entry.background).copied();
        let online = sessions.iter().any(|entry| {
            entry
                .capabilities
                .state()
                .info(cx)
                .is_some_and(|info| info.status != TerminalStatus::Closed)
        });
        let covering = covering(thread.read(cx), summary, &attachment);
        let selected = explicit || covering.is_some();
        let id = summary.id.to_string();

        // Indented with padding, not margin: a margin beside a full width
        // pushed the open button past the menu's edge.
        let line = h_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .when(nested, |row| {
                // A guide down from the folder, as in the sidebar.
                row.pl_1().border_l_1().border_color(cx.theme().border)
            })
            .child(
                menu_row_with(
                    SharedString::from(format!("attach-connection-{id}")),
                    summary.name.clone(),
                    summary.icon.clone().map(server_image),
                    cx,
                )
                .when(summary.icon.is_none(), |row| {
                    row.icon(attachment_icon(&attachment))
                })
                .when(online, |row| row.child(running_dot(cx)))
                .when_some(summary.flag.clone(), |row, flag| {
                    row.child(flag_image(flag))
                })
                .tooltip(match &covering {
                    Some(through) => format!("{} · attached with {through}", summary.name),
                    None if online => format!("{} · open", summary.name),
                    None => format!("{} · agents connect in the background", summary.name),
                })
                .flex_1()
                .min_w_0()
                .selected(selected)
                .when(explicit || covering.is_none(), |row| {
                    row.on_click(cx.listener(move |this, _, _, cx| {
                        for attachment in &toggles {
                            this.on_attachment_selected(origin, attachment.clone(), cx);
                        }
                    }))
                }),
            )
            .when_some(background, |line, entry| {
                line.child(Self::show_background(
                    entry.item,
                    thread.read(cx).workspace_seat().clone(),
                    cx,
                ))
            })
            .when(background.is_none(), |line| {
                line.child(SavedServerOpen::button(summary, opening, cx))
            });
        h_flex()
            .w_full()
            .when(nested, |row| row.pl(px(14.)))
            .child(line)
            .into_any_element()
    }

    fn show_background(
        item: gpui_kit::EntityId,
        seat: Seat,
        cx: &mut Context<AgentPanel>,
    ) -> Button {
        Button::new(SharedString::from(format!("show-background-{item:?}")))
            .custom(menu_variant(cx))
            .small()
            .flex_shrink_0()
            .label("Show")
            .tooltip("Opened by an agent in the background. Show it in a tab.")
            .on_click(cx.listener(move |panel, _, window, cx| {
                panel.close_menus(cx);
                let seat = seat.clone();
                window.defer(cx, move |window, cx| seat.show_background(item, window, cx));
                cx.notify();
            }))
    }
}

pub(super) struct AttachmentMenuRenderer;

/// The arrow retains the original destination; General keeps its recent-use path.
#[derive(Clone)]
enum SavedServerOpen {
    General {
        directory: Rc<dyn ConnectionDirectory>,
        workspace: gpui_kit::WeakEntity<nocterm_workspace::Workspace>,
    },
    Workspace(Seat),
}
impl SavedServerOpen {
    fn button(
        summary: &ConnectionSummary,
        opening: Option<Self>,
        cx: &mut Context<AgentPanel>,
    ) -> Button {
        let id = summary.id.to_string();
        Button::new(SharedString::from(format!("open-connection-{id}")))
            .custom(menu_variant(cx))
            .small()
            .flex_shrink_0()
            .disabled(opening.is_none())
            .icon(IconName::ArrowUpRight)
            .accessibility_label(format!("Open {} in a new tab", summary.name))
            .tooltip("Open in a new tab")
            .on_click(cx.listener(move |panel, _, window, cx| {
                panel.close_menus(cx);
                let opening = opening.clone();
                let id = id.clone();
                // Opening updates the workspace; wait until panel rendering is over.
                window.defer(cx, move |window, cx| {
                    if let Some(opening) = opening {
                        opening.open(&id, window, cx);
                    }
                });
                cx.notify();
            }))
    }

    fn open(&self, profile: &str, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
        match self {
            Self::General {
                directory,
                workspace,
            } => {
                directory.open(profile, workspace, window, cx);
            }
            Self::Workspace(seat) => {
                if let Some(item) = seat.open_background(profile, window, cx) {
                    seat.show_background(item, window, cx);
                }
            }
        }
    }
}

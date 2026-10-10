//! The attach menu: what a chat's agent may work with.
//!
//! Two parts. *Active sessions* are terminals that are open now: the user's
//! tabs, the bottom shell, and sessions agents opened in the background.
//! *Saved servers* are connections, laid out as in the sidebar; attaching
//! one (or its folder) lets the agent use its open sessions, or open one in
//! the background when there is none.
use std::rc::Rc;

use gpui_kit::{
    AnyElement, Context, Entity, SharedString, TestSupportExt as _,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
    px,
};
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

impl AgentPanel {
    pub(super) fn render_attach_menu(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (Some(thread), Some(workspace)) = (self.current(), self.workspace.upgrade()) else {
            return div().into_any_element();
        };
        let directory = workspace.read(cx).connection_directory();
        let summaries = directory
            .as_ref()
            .map(|directory| directory.connections(cx))
            .unwrap_or_default();
        let terminals = workspace.read(cx).terminals(cx);
        let mut menu = v_flex()
            .w_full()
            .gap(px(2.))
            .child(heading("Active sessions", cx));
        if terminals.is_empty() {
            menu = menu.child(empty("No open terminals.", cx));
        }
        for entry in &terminals {
            menu = menu.child(self.terminal_row(entry, &summaries, &thread, cx));
        }
        if let Some(directory) = directory
            && !summaries.is_empty()
        {
            menu = menu.child(heading("Saved servers", cx));
            for row in servers::server_rows(&summaries) {
                menu = menu.child(match row {
                    servers::ServerRow::Folder(group) => self.folder_row(group, &thread, cx),
                    servers::ServerRow::Server { summary, nested } => {
                        let online = terminals.iter().any(|entry| {
                            entry.access.info(cx).is_some_and(|info| {
                                info.profile.as_ref() == Some(&summary.id)
                                    && info.status != TerminalStatus::Closed
                            })
                        });
                        self.server_row(summary, nested, online, &thread, &directory, cx)
                    }
                });
            }
        }
        menu.into_any_element()
    }

    /// An open terminal. A background one can be brought into a tab.
    fn terminal_row(
        &self,
        entry: &TerminalEntry,
        summaries: &[ConnectionSummary],
        thread: &Entity<AgentThread>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let attachment = Attachment::Terminal(entry.item);
        let info = entry.access.info(cx);
        // A saved server's session is attached through the server as well.
        let summary = info
            .as_ref()
            .and_then(|info| info.profile.as_ref())
            .and_then(|profile| summaries.iter().find(|summary| summary.id == *profile));
        let explicit = thread.read(cx).composer.attachments.contains(&attachment);
        let covering = summary.and_then(|summary| covering(thread.read(cx), summary, &attachment));
        let selected = explicit || covering.is_some();
        let icon = summary.and_then(|summary| summary.icon.clone());
        let connected = info
            .as_ref()
            .is_some_and(|info| info.status == TerminalStatus::Connected);
        let item = entry.item;
        let row = menu_row_with(
            SharedString::from(format!("attach-{:?}", entry.item)),
            entry.title.clone(),
            icon.clone().map(server_image),
            cx,
        )
        .when(icon.is_none(), |row| row.icon(attachment_icon(&attachment)))
        .when(connected, |row| row.child(running_dot(cx)))
        .flex_1()
        .min_w_0()
        .selected(selected)
        .when_some(covering.clone(), |row, through| {
            row.tooltip(format!("{} · attached with {through}", entry.title))
        })
        .when(explicit || covering.is_none(), |row| {
            row.on_click(cx.listener(move |this, _, _, cx| {
                if let Some(thread) = this.current() {
                    thread.update(cx, |thread, cx| thread.attach(attachment.clone(), cx));
                }
            }))
        });
        let mut line = h_flex().w_full().gap_1().child(row);
        if entry.background {
            let workspace = self.workspace.clone();
            line = line.child(
                Button::new(SharedString::from(format!("show-background-{item:?}")))
                    .custom(menu_variant(cx))
                    .small()
                    .label("Show")
                    .tooltip("Opened by an agent in the background. Show it in a tab.")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.menu = None;
                        let workspace = workspace.clone();
                        window.defer(cx, move |window, cx| {
                            let _ = workspace.update(cx, |workspace, cx| {
                                workspace.show_background_session(item, window, cx)
                            });
                        });
                        cx.notify();
                    })),
            );
        }
        line.into_any_element()
    }

    /// A folder; attaching it attaches every server in it.
    fn folder_row(
        &self,
        group: &str,
        thread: &Entity<AgentThread>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let attachment = Attachment::Group(group.to_owned());
        let selected = thread.read(cx).composer.attachments.contains(&attachment);
        menu_row(
            SharedString::from(format!("attach-group-{group}")),
            group.to_owned(),
            cx,
        )
        .icon(attachment_icon(&attachment))
        .tooltip(format!("Attach every server in {group}"))
        .selected(selected)
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(thread) = this.current() {
                thread.update(cx, |thread, cx| thread.attach(attachment.clone(), cx));
            }
        }))
        .into_any_element()
    }

    /// A saved server, indented under its folder. The arrow opens it in a
    /// tab, as from the sidebar.
    fn server_row(
        &self,
        summary: &ConnectionSummary,
        nested: bool,
        online: bool,
        thread: &Entity<AgentThread>,
        directory: &Rc<dyn ConnectionDirectory>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let attachment = Attachment::Connection(summary.id.to_string());
        let explicit = thread.read(cx).composer.attachments.contains(&attachment);
        let covering = covering(thread.read(cx), summary, &attachment);
        let selected = explicit || covering.is_some();
        let id = summary.id.to_string();
        let directory = directory.clone();
        let workspace = self.workspace.clone();
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
                        if let Some(thread) = this.current() {
                            thread.update(cx, |thread, cx| thread.attach(attachment.clone(), cx));
                        }
                    }))
                }),
            )
            .child(
                Button::new(SharedString::from(format!("open-connection-{id}")))
                    .custom(menu_variant(cx))
                    .small()
                    .flex_shrink_0()
                    .icon(IconName::ArrowUpRight)
                    .accessibility_label(format!("Open {} in a new tab", summary.name))
                    .tooltip("Open in a new tab")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.menu = None;
                        let directory = directory.clone();
                        let workspace = workspace.clone();
                        let id = id.clone();
                        // Opening updates the workspace, which may be
                        // rendering this panel: do it once that is over.
                        window.defer(cx, move |window, cx| {
                            directory.open(&id, &workspace, window, cx);
                        });
                        cx.notify();
                    })),
            );
        h_flex()
            .w_full()
            .when(nested, |row| row.pl(px(14.)))
            .child(line)
            .into_any_element()
    }
}

/// Another of the chat's attachments that already gives the agent `summary`,
/// named for a row shown selected through it. Such a row does not toggle: a
/// click would only add a duplicate that outlives the broader attachment.
fn covering(thread: &AgentThread, summary: &ConnectionSummary, own: &Attachment) -> Option<String> {
    thread
        .composer
        .attachments
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

fn empty(text: &'static str, cx: &Context<AgentPanel>) -> AnyElement {
    div()
        .px_2()
        .py_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

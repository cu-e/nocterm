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
        let through_server = summary.is_some_and(|summary| {
            thread
                .read(cx)
                .composer
                .attachments
                .iter()
                .any(|attachment| match attachment {
                    Attachment::Connection(id) => summary.id.as_ref() == id,
                    Attachment::Group(group) => summary
                        .group
                        .as_ref()
                        .is_some_and(|name| name.as_ref() == group),
                    Attachment::Terminal(_) | Attachment::UnavailableLocal(_) => false,
                })
        });
        let selected = thread.read(cx).composer.attachments.contains(&attachment) || through_server;
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
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(thread) = this.current() {
                thread.update(cx, |thread, cx| thread.attach(attachment.clone(), cx));
            }
        }));
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

    /// A saved server, indented under its folder. Open shows it in a tab, as
    /// from the sidebar.
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
        let in_group = summary.group.as_ref().is_some_and(|group| {
            thread
                .read(cx)
                .composer
                .attachments
                .contains(&Attachment::Group(group.to_string()))
        });
        let selected = thread.read(cx).composer.attachments.contains(&attachment) || in_group;
        let id = summary.id.to_string();
        let directory = directory.clone();
        let workspace = self.workspace.clone();
        h_flex()
            .w_full()
            .gap_1()
            .when(nested, |row| {
                // A guide down from the folder, as in the sidebar.
                row.ml(px(14.))
                    .pl_1()
                    .border_l_1()
                    .border_color(cx.theme().border)
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
                .tooltip(if online {
                    format!("{} · open", summary.name)
                } else {
                    format!("{} · agents connect in the background", summary.name)
                })
                .flex_1()
                .min_w_0()
                .selected(selected)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(thread) = this.current() {
                        thread.update(cx, |thread, cx| thread.attach(attachment.clone(), cx));
                    }
                })),
            )
            .child(
                Button::new(SharedString::from(format!("open-connection-{id}")))
                    .custom(menu_variant(cx))
                    .small()
                    .label("Open")
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
            )
            .into_any_element()
    }
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

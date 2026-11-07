//! The servers the hub's agents sign in to that this window has not saved: connected in the web
//! panel or from another device. Choosing one opens a new connection to it with the address
//! filled in; once saved it is an ordinary saved server whose row shows what agents found.
use super::*;
use crate::{AgentServer, UnsavedAgentServer, editor::open_editor_for};

impl ConnectionsPanel {
    /// The section under the saved servers, or nothing when every agent server is saved here.
    pub(super) fn render_agent_servers(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let access = self.installed.inputs().agent_access()?;
        let saved: Vec<AgentServer> = self
            .installed
            .connections()
            .read(cx)
            .profiles(cx)
            .iter()
            .map(AgentServer::of)
            .collect();
        let unsaved = access.unsaved(&saved, cx);
        if unsaved.is_empty() {
            return None;
        }
        let rows: Vec<AnyElement> = unsaved
            .into_iter()
            .enumerate()
            .map(|(index, server)| self.render_agent_server(index, server, cx))
            .collect();
        let theme = cx.theme();
        let header = div()
            .px_2()
            .pt_2()
            .pb_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child("Agent servers not saved here");
        Some(
            v_flex()
                .id("agent-servers")
                .test_support()
                .flex_shrink_0()
                .border_t_1()
                .border_color(theme.border)
                .child(header)
                .children(rows)
                .into_any_element(),
        )
    }

    fn render_agent_server(
        &self,
        index: usize,
        server: UnsavedAgentServer,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let state = server.state;
        let target = format!("{}@{}:{}", server.user, server.host, server.port);
        let name = if server.name.is_empty() {
            server.host.clone()
        } else {
            server.name.clone()
        };
        let accent = theme.accent;
        h_flex()
            .id(("agent-server", index))
            .test_support()
            .mx_1()
            .px_2()
            .py_1()
            .gap_2()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(move |row| row.bg(accent))
            .tooltip(|_, cx| {
                cx.new(|_| {
                    gpui_kit::component::tooltip::Tooltip::new(
                        "Save it here to open it on this device",
                    )
                })
                .into()
            })
            .child(
                Icon::new(IconName::Server)
                    .small()
                    .text_color(theme.muted_foreground),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().truncate().child(name))
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(target),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(if state.is_ready() {
                        theme.muted_foreground
                    } else {
                        theme.warning
                    })
                    .child(state.label()),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                open_editor_for(
                    this.installed.clone(),
                    server.clone(),
                    this.workspace.clone(),
                    window,
                    cx,
                );
            }))
            .into_any_element()
    }
}

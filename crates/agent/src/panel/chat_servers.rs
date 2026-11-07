//! The servers an agent that runs elsewhere may sign in to, as its chat offers them.
//!
//! Such an agent gets none of this device's terminals: the attach menu lists the servers the
//! chat offers, each checked while the chat has it attached, and the composer shows the
//! attached ones as removable chips. Every server is a flag of the chat's configuration
//! (`ConfigCategory::Server`), so a choice is stored where the agent runs and every device
//! shows it.

use gpui_kit::{
    AnyElement, Context, Entity, SharedString,
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_ai::model::controls::{ConfigCategory, ConfigKind, ConfigOption, ConfigSetting};
use nocterm_ui::IconName;

use super::{
    AgentPanel,
    widgets::{menu_row, menu_variant},
};
use crate::thread::AgentThread;

/// A server the chat offers, and whether the chat has it attached.
struct Offered {
    option: ConfigOption,
    attached: bool,
}

fn offered(thread: &AgentThread) -> Vec<Offered> {
    thread
        .config_options()
        .into_iter()
        .filter(is_server)
        .map(|option| Offered {
            attached: matches!(option.kind, ConfigKind::Boolean(true)),
            option,
        })
        .collect()
}

/// Whether `option` attaches a server rather than configuring the agent.
pub(super) fn is_server(option: &ConfigOption) -> bool {
    option.category == Some(ConfigCategory::Server)
}

/// Whether the chat's agent signs in to the chat's servers instead of using this device's
/// terminals.
pub(super) fn takes_servers(thread: &AgentThread) -> bool {
    thread
        .agent_info()
        .is_some_and(|info| info.capabilities.own_tools)
        || thread.config_options().iter().any(is_server)
}

/// Attaches or detaches the server of `option` through the chat's configuration.
fn choose(
    thread: &Entity<AgentThread>,
    option: &ConfigOption,
    attach: bool,
    cx: &mut Context<AgentPanel>,
) {
    let id = option.id.clone();
    thread.update(cx, |thread, cx| {
        thread.set_config(id, ConfigSetting::Boolean(attach), cx);
    });
}

pub(super) struct ChatServersRenderer;

impl ChatServersRenderer {
    /// The attach menu of a chat whose agent takes servers.
    pub(super) fn render_menu(
        thread: &Entity<AgentThread>,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let servers = offered(thread.read(cx));
        let busy = thread.read(cx).session_generating();
        let mut menu = v_flex().w_full().gap(px(2.));
        if servers.is_empty() {
            menu = menu.child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "No servers yet. Give agents access to a saved server, or connect one in \
                         the hub's web panel.",
                    ),
            );
        }
        for (index, server) in servers.into_iter().enumerate() {
            menu = menu.child(Self::row(thread, index, server, busy, cx));
        }
        menu.into_any_element()
    }

    fn row(
        thread: &Entity<AgentThread>,
        index: usize,
        server: Offered,
        busy: bool,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let Offered { option, attached } = server;
        let address = option.description.clone().unwrap_or_default();
        let theme = cx.theme().clone();
        let thread = thread.clone();
        let chosen = option.clone();
        menu_row(("attach-server", index), option.name.clone(), cx)
            .icon(if attached {
                IconName::Check
            } else {
                IconName::Server
            })
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(px(180.))
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(address.clone()),
            )
            .tooltip(format!("{} · {address}", option.name))
            .selected(attached)
            .disabled(busy)
            .on_click(cx.listener(move |_, _, _, cx| {
                choose(&thread, &chosen, !attached, cx);
            }))
            .into_any_element()
    }

    /// Removable chips for the servers the chat has attached.
    pub(super) fn chips(
        thread: &Entity<AgentThread>,
        cx: &mut Context<AgentPanel>,
    ) -> Vec<AnyElement> {
        let busy = thread.read(cx).session_generating();
        offered(thread.read(cx))
            .into_iter()
            .filter(|server| server.attached)
            .enumerate()
            .map(|(index, server)| {
                let thread = thread.clone();
                let option = server.option;
                let name = option.name.clone();
                Button::new(SharedString::from(format!("server-chip-{index}")))
                    .custom(menu_variant(cx))
                    .small()
                    .icon(IconName::Server)
                    .max_w_full()
                    .label(name.clone())
                    .tooltip(format!(
                        "Detach {name} · {}",
                        option.description.clone().unwrap_or_default()
                    ))
                    .disabled(busy)
                    .child(gpui_kit::component::Icon::new(IconName::X).xsmall())
                    .on_click(cx.listener(move |_, _, _, cx| {
                        choose(&thread, &option, false, cx);
                    }))
                    .into_any_element()
            })
            .collect()
    }
}

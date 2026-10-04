//! Requests waiting for the user: the agent's own permission prompts and
//! terminal tool calls that need approval.
use gpui_kit::{
    AnyElement, Context, Entity, SharedString, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_ai::TerminalCall;
use nocterm_ui::IconName;
use nocterm_workspace::UnlockVault;

use super::{AgentPanel, widgets::menu_variant};
use crate::thread::AgentThread;

impl AgentPanel {
    /// The cards of every pending request of `thread`; none when nothing waits.
    pub(super) fn render_approvals(
        &mut self,
        thread: &Entity<AgentThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let pending = thread.read(cx).permissions.len()
            + thread.read(cx).tools.len()
            + thread.read(cx).vault_waits.len();
        if pending == 0 {
            return None;
        }
        let mut list = v_flex()
            .id("agent-approval-list")
            .test_support()
            .w_full()
            .min_w_0()
            .min_h_0()
            .max_h(window.viewport_size().height * 0.35)
            .overflow_y_scroll()
            .gap_2();
        for index in 0..thread.read(cx).vault_waits.len() {
            list = list.child(self.vault_card(thread, index, cx));
        }
        for index in 0..thread.read(cx).permissions.len() {
            list = list.child(self.permission_card(thread, index, cx));
        }
        for index in 0..thread.read(cx).tools.len() {
            list = list.child(self.tool_card(thread, index, cx));
        }
        Some(
            v_flex()
                .id("agent-approvals")
                .test_support()
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .px_3()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().warning)
                        .child(format!("Waiting for you · {pending}")),
                )
                .child(list)
                .into_any_element(),
        )
    }

    /// A background session needs a saved secret from the locked vault.
    /// Unlocking answers its sign-in prompt; the session never becomes a tab
    /// unless the user chooses to sign in by hand.
    fn vault_card(
        &self,
        thread: &Entity<AgentThread>,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (item, title) = thread.read(cx).vault_waits[index].clone();
        let workspace = thread.read(cx).workspace.clone();
        card(cx)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .child("Unlock the vault to sign in"),
            )
            .child(
                div()
                    .min_w_0()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{title} uses a password saved in the vault. The agent continues once it is unlocked."
                    )),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .gap_1()
                    .child(
                        Button::new(("vault-unlock", index))
                            .primary()
                            .small()
                            .icon(IconName::Lock)
                            .label("Unlock vault")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(UnlockVault), cx)
                            }),
                    )
                    .child(
                        Button::new(("vault-sign-in-manually", index))
                            .custom(menu_variant(cx))
                            .small()
                            .label("Enter password in terminal")
                            .on_click(move |_, window, cx| {
                                let _ = workspace.update(cx, |workspace, cx| {
                                    workspace.show_background_session(item, window, cx)
                                });
                            }),
                    ),
            )
            .into_any_element()
    }

    fn permission_card(
        &self,
        thread: &Entity<AgentThread>,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let permission = &thread.read(cx).permissions[index];
        let title = permission
            .request
            .tool_call
            .fields
            .title
            .clone()
            .unwrap_or_else(|| "Agent permission request".into());
        let options: Vec<_> = permission
            .request
            .options
            .iter()
            .map(|option| (option.option_id.clone(), option.name.clone()))
            .collect();
        let mut actions = h_flex().w_full().min_w_0().flex_wrap().gap_1();
        for (id, name) in options {
            actions = actions.child(
                Button::new(SharedString::from(format!("permission-{index}-{id}")))
                    .custom(menu_variant(cx))
                    .small()
                    .max_w_full()
                    .label(name)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(thread) = this.current() {
                            thread.update(cx, |thread, cx| {
                                thread.choose_permission(index, Some(id.clone()), cx)
                            });
                        }
                    })),
            );
        }
        actions = actions.child(
            Button::new(("permission-cancel", index))
                .custom(menu_variant(cx))
                .small()
                .label("Cancel")
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(thread) = this.current() {
                        thread.update(cx, |thread, cx| thread.choose_permission(index, None, cx));
                    }
                })),
        );
        card(cx)
            .child(div().w_full().min_w_0().truncate().text_sm().child(title))
            .child(actions)
            .into_any_element()
    }

    fn tool_card(
        &self,
        thread: &Entity<AgentThread>,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let call = thread.read(cx).tools[index].call.clone();
        let target = thread.update(cx, |thread, cx| thread.describe_target(&call, cx));
        let (heading, exact, grant) = match &call {
            TerminalCall::ReadTerminal(request) => (
                "Read terminal output",
                format!("{target} ({} lines)", request.lines.unwrap_or(200)),
                "Allow for this terminal",
            ),
            TerminalCall::SendInput(request) => (
                "Type into terminal",
                format!(
                    "{target}: {:?}{}",
                    request.text,
                    if request.press_enter { " + Enter" } else { "" }
                ),
                "Allow for this terminal",
            ),
            TerminalCall::RunCommand(request) => (
                "Run command",
                format!("{target}: {}", request.command),
                "Allow for this terminal",
            ),
            TerminalCall::OpenTerminal(_) => {
                ("Connect in the background", target, "Allow for this server")
            }
            TerminalCall::ListTerminals => ("List terminals", String::new(), "Allow"),
        };
        card(cx)
            .child(div().min_w_0().truncate().text_sm().child(heading))
            .child(
                h_flex().w_full().min_w_0().flex_wrap().gap_1().children(
                    [
                        (false, false, "Deny"),
                        (true, false, "Allow once"),
                        (true, true, grant),
                    ]
                    .into_iter()
                    .map(|(allow, grant, label)| {
                        Button::new((SharedString::from(format!("tool-{label}")), index))
                            .custom(menu_variant(cx))
                            .small()
                            .max_w_full()
                            .label(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(thread) = this.current() {
                                    thread.update(cx, |thread, cx| {
                                        thread.approve_tool(index, allow, grant, cx)
                                    });
                                }
                            }))
                    }),
                ),
            )
            .child(
                div()
                    .id(("approval-command", index))
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .max_h(px(64.))
                    .overflow_x_scroll()
                    .overflow_y_scroll()
                    .text_xs()
                    .child(exact),
            )
            .into_any_element()
    }
}

fn card(cx: &Context<AgentPanel>) -> gpui_kit::Div {
    v_flex()
        .w_full()
        .min_w_0()
        .p_2()
        .gap_1()
        .rounded(px(8.))
        .bg(cx.theme().muted)
}

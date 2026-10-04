//! Requests waiting for the user: the agent's own permission prompts and
//! terminal tool calls that need approval.
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, EntityId, SharedString, Subscription,
    TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
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

/// The password field of a sign-in card.
pub(super) struct SignInInput {
    pub(super) input: Entity<InputState>,
    masked: bool,
    _submit: Subscription,
}

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
            + thread.read(cx).sign_ins.len();
        let asking: Vec<EntityId> = self
            .threads
            .iter()
            .flat_map(|thread| thread.read(cx).sign_ins.iter().map(|wait| wait.item))
            .collect();
        self.sign_in_inputs.retain(|item, _| asking.contains(item));
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
        for index in 0..thread.read(cx).sign_ins.len() {
            list = list.child(self.sign_in_card(thread, index, window, cx));
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

    /// A background session asks for a secret. The user types it here, or
    /// unlocks the vault when a saved secret answers it; the session never
    /// becomes a tab.
    fn sign_in_card(
        &mut self,
        thread: &Entity<AgentThread>,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let wait = thread.read(cx).sign_ins[index].clone();
        let item = wait.item;
        let input = self.sign_in_input(thread, item, wait.prompt.masked, window, cx);
        let heading = if wait.vault {
            "Unlock the vault to sign in".to_owned()
        } else if wait.prompt.retry {
            format!("That was not accepted. Sign in to {} again", wait.title)
        } else {
            format!("Sign in to {}", wait.title)
        };
        let detail = if wait.vault {
            format!(
                "{} uses a password saved in the vault. Unlock it, or type the password here.",
                wait.title
            )
        } else {
            format!(
                "{}. The agent continues once you are signed in.",
                wait.prompt.label
            )
        };
        let field = Input::new(&input).small();
        let field = if wait.prompt.masked {
            field.mask_toggle()
        } else {
            field
        };
        let panel = cx.weak_entity();
        let weak_thread = thread.downgrade();
        card(cx)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .when(wait.prompt.retry && !wait.vault, |text| {
                        text.text_color(cx.theme().danger)
                    })
                    .child(heading),
            )
            .child(
                div()
                    .min_w_0()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(detail),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .when(wait.vault, |row| {
                        row.child(
                            Button::new(("vault-unlock", index))
                                .primary()
                                .small()
                                .icon(IconName::Lock)
                                .label("Unlock vault")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(UnlockVault), cx)
                                }),
                        )
                    })
                    .child(div().flex_1().min_w_0().child(field))
                    .child(
                        Button::new(("sign-in-submit", index))
                            .when(wait.vault, |button| button.custom(menu_variant(cx)))
                            .when(!wait.vault, |button| button.primary())
                            .small()
                            .label("Sign in")
                            .on_click(move |_, window, cx| {
                                let _ = panel.update(cx, |panel, cx| {
                                    panel.submit_sign_in(&weak_thread, item, window, cx)
                                });
                            }),
                    ),
            )
            .into_any_element()
    }

    fn sign_in_input(
        &mut self,
        thread: &Entity<AgentThread>,
        item: EntityId,
        masked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(field) = self.sign_in_inputs.get(&item)
            && field.masked == masked
        {
            return field.input.clone();
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(masked)
                .placeholder(if masked { "Password" } else { "Answer" })
        });
        let weak_thread = thread.downgrade();
        let submit = cx.subscribe_in(&input, window, move |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_sign_in(&weak_thread, item, window, cx);
            }
        });
        self.sign_in_inputs.insert(
            item,
            SignInInput {
                input: input.clone(),
                masked,
                _submit: submit,
            },
        );
        input
    }

    /// Answers the session's prompt with what the user typed. The card goes
    /// away when the session stops asking.
    pub(super) fn submit_sign_in(
        &mut self,
        thread: &gpui_kit::WeakEntity<AgentThread>,
        item: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = self.sign_in_inputs.get(&item) else {
            return;
        };
        let answer = field.input.read(cx).value().to_string();
        if answer.is_empty() {
            return;
        }
        field
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        let access = thread.upgrade().and_then(|thread| {
            let thread = thread.read(cx);
            let wait = thread.sign_ins.iter().find(|wait| wait.item == item)?;
            Some(wait.access.clone())
        });
        if let Some(access) = access
            && let Err(error) = access.answer_sign_in(answer, cx)
        {
            self.error = Some(error);
        }
        cx.notify();
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

//! Requests waiting for the user: the agent's own permission prompts and
//! terminal tool calls that need approval.
use gpui_kit::{
    AnyElement, Context, Entity, SharedString, TestSupportExt as _,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_ai::{TerminalCall, TransferFiles};
use nocterm_ui::IconName;
use nocterm_workspace::UnlockVault;

use super::{AgentPanel, widgets::menu_variant};
use crate::thread::{AgentThread, PermissionCard};
use nocterm_agent_chat::conversation::{ConversationToolApproval, ToolApprovalChoice};
use std::rc::Rc;

fn sign_in_text(wait: &crate::thread::SignInWait) -> (String, String) {
    let heading = if wait.vault() {
        "Unlock the vault to sign in".to_owned()
    } else if wait.prompt().retry {
        format!("That was not accepted. Sign in to {} again", wait.title())
    } else {
        format!("Sign in to {}", wait.title())
    };
    let detail = if wait.vault() {
        format!(
            "{} uses a password saved in the vault. Unlock it, or type the password here.",
            wait.title()
        )
    } else {
        format!(
            "{}. The agent continues once you are signed in.",
            wait.prompt().label
        )
    };
    (heading, detail)
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

pub(super) struct ApprovalCardsRenderer;
impl ApprovalCardsRenderer {
    pub(super) fn list(
        cards: Vec<AnyElement>,
        window: &gpui_kit::Window,
        cx: &Context<AgentPanel>,
    ) -> Option<AnyElement> {
        let pending = cards.len();
        if pending == 0 {
            return None;
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
                .child(
                    v_flex()
                        .id("agent-approval-list")
                        .test_support()
                        .w_full()
                        .min_w_0()
                        .min_h_0()
                        .max_h(window.viewport_size().height * 0.35)
                        .overflow_y_scroll()
                        .gap_2()
                        .children(cards),
                )
                .into_any_element(),
        )
    }
    /// A background session asks for a secret. The user types it here, or
    /// unlocks the vault when a saved secret answers it; the session never
    /// becomes a tab.
    pub(super) fn sign_in_card(
        wait: crate::thread::SignInWait,
        input: Entity<InputState>,
        submission: super::sign_in::SignInSubmission,
        index: usize,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let (heading, detail) = sign_in_text(&wait);
        let field = Input::new(&input).id(("sign-in-input", index)).small();
        let field = if wait.prompt().masked {
            field.mask_toggle()
        } else {
            field
        };
        let panel = cx.weak_entity();
        card(cx)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .when(wait.prompt().retry && !wait.vault(), |text| {
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
                    .when(wait.vault(), |row| {
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
                            .when(wait.vault(), |button| button.custom(menu_variant(cx)))
                            .when(!wait.vault(), |button| button.primary())
                            .small()
                            .label("Sign in")
                            .on_click(move |_, window, cx| {
                                let _ = panel.update(cx, |panel, cx| {
                                    panel.submit_sign_in(&submission, window, cx)
                                });
                            }),
                    ),
            )
            .into_any_element()
    }
    pub(super) fn permission_card(
        thread: &Entity<AgentThread>,
        permission: PermissionCard,
        index: usize,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let generation = permission.token();
        let title = permission
            .request()
            .tool
            .changes
            .title
            .clone()
            .unwrap_or_else(|| "Agent permission request".into());
        let options: Vec<_> = permission
            .request()
            .choices
            .iter()
            .map(|option| (option.id.clone(), option.label.clone()))
            .collect();
        let mut actions = h_flex().w_full().min_w_0().flex_wrap().gap_1();
        for (id, name) in options {
            let owner = thread.downgrade();
            actions = actions.child(
                Button::new(SharedString::from(format!("permission-{index}-{id}")))
                    .custom(menu_variant(cx))
                    .small()
                    .max_w_full()
                    .label(name)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        if let Some(thread) = owner.upgrade() {
                            thread.update(cx, |thread, cx| {
                                thread.choose_permission_request(generation, Some(id.clone()), cx)
                            });
                        }
                    })),
            );
        }
        let owner = thread.downgrade();
        actions = actions.child(
            Button::new(("permission-cancel", index))
                .custom(menu_variant(cx))
                .small()
                .label("Cancel")
                .on_click(cx.listener(move |_, _, _, cx| {
                    if let Some(thread) = owner.upgrade() {
                        thread.update(cx, |thread, cx| {
                            thread.choose_permission_request(generation, None, cx)
                        });
                    }
                })),
        );
        card(cx)
            .child(div().w_full().min_w_0().truncate().text_sm().child(title))
            .when_some(permission.explanation(), |card, explanation| {
                card.child(
                    div()
                        .id(("permission-explanation", index))
                        .test_support()
                        .w_full()
                        .min_w_0()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(explanation),
                )
            })
            .child(actions)
            .into_any_element()
    }
    pub(super) fn tool_card(
        approval: Rc<dyn ConversationToolApproval>,
        identity: gpui_kit::EntityId,
        index: usize,
        cx: &mut Context<AgentPanel>,
    ) -> Option<AnyElement> {
        let mut details = None;
        approval
            .with_view(cx, &mut |view| details = Some(Self::tool_details(view)))
            .ok()?;
        let (heading, exact, grant, reusable) = details?;
        let exact = nocterm_ai::redact::redact(&exact);
        Some(
            card(cx)
                .child(div().min_w_0().truncate().text_sm().child(heading))
                .child(
                    h_flex().w_full().min_w_0().flex_wrap().gap_1().children(
                        [
                            (ToolApprovalChoice::Deny, false, "Deny"),
                            (ToolApprovalChoice::AllowOnce, false, "Allow once"),
                            (ToolApprovalChoice::AllowForSession, true, grant),
                        ]
                        .into_iter()
                        .filter(|(_, grant, _)| !grant || reusable)
                        .map(|(choice, _, label)| {
                            let approval = approval.clone();
                            Button::new((SharedString::from(format!("tool-{label}")), index))
                                .custom(menu_variant(cx))
                                .small()
                                .max_w_full()
                                .label(label)
                                .on_click(cx.listener(move |panel, _, _, cx| {
                                    panel.choose_conversation_tool(
                                        identity,
                                        approval.clone(),
                                        choice,
                                        cx,
                                    );
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
                .into_any_element(),
        )
    }
    fn tool_details(
        view: nocterm_agent_chat::conversation::ToolApprovalView<'_>,
    ) -> (&'static str, String, &'static str, bool) {
        let (heading, mut exact, grant) = Self::tool_text(view.call(), view.target());
        if let Some(principal) = view.principal() {
            exact.push_str(&format!("\nPrincipal: {principal}"));
        }
        if let Some(expires) = view.expires_ms() {
            let date = i64::try_from(expires)
                .ok()
                .and_then(chrono::DateTime::from_timestamp_millis);
            match date {
                Some(date) => exact.push_str(&format!(
                    "\nExpires: {}",
                    date.format("%Y-%m-%d %H:%M:%S UTC")
                )),
                None => exact.push_str("\nExpiry cannot be displayed"),
            }
        }
        (heading, exact, grant, view.reusable())
    }
    fn tool_text(call: &TerminalCall, target: &str) -> (&'static str, String, &'static str) {
        match call {
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
                "Allow input at an empty prompt",
            ),
            TerminalCall::RunCommand(request) => (
                "Run command",
                format!("{target}: {}", request.command),
                "Allow for this terminal",
            ),
            TerminalCall::ExecCommand(request) => (
                "Execute program",
                format!(
                    "{target}\nProgram: {}\nArguments: {:?}\nTimeout: {} ms · stdin: {} bytes{}",
                    request.program,
                    request.args,
                    request.timeout_ms.unwrap_or(30_000),
                    request.stdin.as_ref().map_or(0, String::len),
                    request
                        .stdin
                        .as_ref()
                        .map(|stdin| format!("\nStandard input:\n{stdin}"))
                        .unwrap_or_default()
                ),
                "Allow execution on this terminal",
            ),
            TerminalCall::ReadCommand(request) => (
                "Read command result",
                format!("{target}: {}", request.command_id),
                "Allow reading on this terminal",
            ),
            TerminalCall::CancelCommand(request) => (
                "Cancel command",
                format!("{target}: {}", request.command_id),
                "Allow execution on this terminal",
            ),
            TerminalCall::OpenTerminal(_) => (
                "Connect in the background",
                target.to_owned(),
                "Allow for this server",
            ),
            TerminalCall::ListTerminals => ("List terminals", String::new(), "Allow"),
            TerminalCall::TransferFiles(request) => transfer_text(request, target),
            TerminalCall::ReadTransfer(request) => (
                "Read transfer",
                request.transfer_id.clone(),
                "Allow reading",
            ),
            TerminalCall::CancelTransfer(request) => (
                "Cancel transfer",
                request.transfer_id.clone(),
                "Allow cancelling",
            ),
        }
    }
}

/// `target` names the source and destination terminals of the transfer.
fn transfer_text(request: &TransferFiles, target: &str) -> (&'static str, String, &'static str) {
    let sources: String = request
        .sources
        .iter()
        .map(|source| format!("\n  {source}"))
        .collect();
    let existing = if request.overwrite { "replace" } else { "skip" };
    (
        "Transfer files",
        format!(
            "{target}\nSources:{sources}\nInto: {}\nExisting files: {existing}",
            request.destination
        ),
        "Allow transfers between these terminals",
    )
}

//! The sign-in prompt: a password, passphrase or host question, with the
//! choice to remember the answer in the vault.
use gpui_kit::{
    AnyElement, Context,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::Input,
    },
    div,
    prelude::*,
};
use nocterm_session::SecretRequest;
use nocterm_ui::IconName;
use nocterm_workspace::UnlockVault;

use super::TerminalView;

impl TerminalView {
    pub(super) fn render_secret(
        &mut self,
        request: &SecretRequest,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let input = self.secret.as_ref()?.input.clone();
        let (title, retry, masked) = crate::credentials::describe(request);
        let danger = cx.theme().danger;

        let rememberable = !matches!(request, SecretRequest::Interactive { .. });
        let unlocked = self.terminal.read(cx).vault_unlocked(cx);
        let remember = self.secret.as_ref().is_some_and(|field| field.remember);
        let message = self
            .terminal
            .read(cx)
            .credential_message()
            .map(str::to_owned);
        let field = Input::new(&input);
        let field = if masked { field.mask_toggle() } else { field };

        Some(
            self.card(title, cx)
                .when(retry, |card| {
                    card.child(
                        div()
                            .text_sm()
                            .text_color(danger)
                            .child("That was not accepted. Try again."),
                    )
                })
                .child(field)
                .when_some(message, |card, message| {
                    card.child(div().text_xs().child(message))
                })
                .when(rememberable, |card| {
                    card.child(
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .child(
                                gpui_kit::component::checkbox::Checkbox::new("remember-credential")
                                    .small()
                                    .label("Remember after sign in")
                                    .checked(remember && unlocked)
                                    .disabled(!unlocked)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        if let Some(field) = &mut this.secret {
                                            field.remember = *checked;
                                            cx.notify();
                                        }
                                    })),
                            )
                            .when(!unlocked, |row| {
                                row.child(
                                    Button::new("auth-open-vault")
                                        .link()
                                        .small()
                                        .icon(IconName::Lock)
                                        .label("Unlock vault…")
                                        .on_click(|_, window, cx| {
                                            window.dispatch_action(Box::new(UnlockVault), cx)
                                        }),
                                )
                            }),
                    )
                })
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("secret-cancel")
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_secret(cx))),
                        )
                        .child(
                            Button::new("secret-submit")
                                .primary()
                                .label("Continue")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.submit_secret(window, cx)
                                })),
                        ),
                )
                .into_any_element(),
        )
    }
}

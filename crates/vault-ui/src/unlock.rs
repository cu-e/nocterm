//! Unlocking and locking the vault from anywhere: the command palette, or a
//! sign-in prompt that could use a saved secret.
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, Render, SharedString,
    Subscription, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_session::Secret;
use nocterm_vault::VaultService;
use nocterm_workspace::{LockVault, OpenVault, UnlockVault};
use std::sync::Arc;

use crate::Service;

/// Lets `workspace::UnlockVault` and `workspace::LockVault` work in any
/// window.
pub(crate) fn init(cx: &mut App) {
    cx.on_action(|_: &UnlockVault, cx| {
        if let Some(window) = cx.active_window() {
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| open(window, cx));
            });
        }
    });
    cx.on_action(|_: &LockVault, cx| {
        if let Some(service) = cx.try_global::<Service>() {
            service.0.lock();
        }
        cx.refresh_windows();
    });
}

/// Asks for the master password in a small dialog. A vault that does not
/// exist yet is created in Settings, so that is opened instead.
pub(crate) fn open(window: &mut Window, cx: &mut App) {
    let Some(service) = cx.try_global::<Service>().map(|service| service.0.clone()) else {
        return;
    };
    if service.is_unlocked() {
        return;
    }
    if !service.exists() {
        window.dispatch_action(Box::new(OpenVault), cx);
        return;
    }
    let prompt = cx.new(|cx| UnlockPrompt::new(service, window, cx));
    let focus = prompt.read(cx).focus_handle(cx);
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Unlock the vault")
            .w(px(420.))
            .child(prompt.clone())
    });
    window.focus(&focus, cx);
}

struct UnlockPrompt {
    service: Arc<VaultService>,
    password: Entity<InputState>,
    busy: bool,
    error: Option<SharedString>,
    _subscription: Subscription,
}

impl UnlockPrompt {
    fn new(service: Arc<VaultService>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Master password")
        });
        let subscription = cx.subscribe_in(&password, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit(window, cx);
            }
        });
        Self {
            service,
            password,
            busy: false,
            error: None,
            _subscription: subscription,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.password.read(cx).value().to_string();
        if self.busy || typed.is_empty() {
            return;
        }
        self.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.busy = true;
        self.error = None;
        let future = self.service.unlock(Secret::new(typed));
        cx.spawn_in(window, async move |this, cx| {
            let result = future.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        window.close_dialog(cx);
                        window.refresh();
                    }
                    Err(error) => this.error = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Focusable for UnlockPrompt {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.password.read(cx).focus_handle(cx)
    }
}

impl Render for UnlockPrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .id("vault-unlock-prompt")
            .test_support()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Saved passwords and keys become available to sign-in prompts."),
            )
            .child(Input::new(&self.password).mask_toggle())
            .when_some(self.error.clone(), |prompt, error| {
                prompt.child(div().text_xs().text_color(theme.danger).child(error))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("vault-unlock-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("vault-unlock-submit")
                            .primary()
                            .label(if self.busy { "Unlocking…" } else { "Unlock" })
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}

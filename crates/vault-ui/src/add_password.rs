//! The dialog that adds a password under a name of its own. It signs in to no server until a
//! connection chooses it in its editor.
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, SharedString, Subscription,
    TestSupportExt as _, WeakEntity, Window,
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
use nocterm_ui::form;
use nocterm_vault::{CredentialBinding, VaultAccess};
use std::sync::Arc;

use crate::credentials_tab::CredentialsTab;

pub(crate) struct AddPassword {
    service: Arc<dyn VaultAccess>,
    /// The tab to show the password once it is kept.
    tab: WeakEntity<CredentialsTab>,
    label: Entity<InputState>,
    password: Entity<InputState>,
    busy: bool,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl AddPassword {
    /// Opens the dialog over the vault page.
    pub(crate) fn open(
        service: Arc<dyn VaultAccess>,
        tab: WeakEntity<CredentialsTab>,
        window: &mut Window,
        cx: &mut gpui_kit::App,
    ) {
        let prompt = cx.new(|cx| Self::new(service, tab, window, cx));
        let focus = prompt.read(cx).label.focus_handle(cx);
        window.open_dialog(cx, move |dialog, _, _| {
            let submit = prompt.clone();
            dialog
                .title("Add a password")
                .w(px(420.))
                // Enter reaches the dialog as Confirm; the prompt closes it once the password
                // is kept.
                .on_ok(move |_, window, cx| {
                    submit.update(cx, |prompt, cx| prompt.submit(window, cx));
                    false
                })
                .child(prompt.clone())
        });
        window.focus(&focus, cx);
    }

    fn new(
        service: Arc<dyn VaultAccess>,
        tab: WeakEntity<CredentialsTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let label = cx.new(|cx| InputState::new(window, cx).placeholder("Name, e.g. Office"));
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Password")
        });
        let subscriptions =
            vec![
                cx.subscribe_in(&password, window, |this, _, event, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.submit(window, cx);
                    }
                }),
            ];
        Self {
            service,
            tab,
            label,
            password,
            busy: false,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    /// Keeps the typed password under its name, then closes the dialog.
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let label = self.label.read(cx).value().trim().to_owned();
        let password = self.password.read(cx).value().to_string();
        if label.is_empty() || password.is_empty() {
            self.error = Some("Enter a name and a password.".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        let added = self.service.put(
            None,
            label,
            CredentialBinding::password([]),
            Secret::new(password),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = added.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(_) => {
                        let _ = this.tab.update(cx, |tab, cx| tab.refresh(cx));
                        window.close_dialog(cx);
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

impl Focusable for AddPassword {
    fn focus_handle(&self, cx: &gpui_kit::App) -> FocusHandle {
        self.label.focus_handle(cx)
    }
}

impl Render for AddPassword {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .id("vault-add-dialog")
            .test_support()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Choose it for a connection under Authentication, Saved passwords. One password can sign in to several servers."),
            )
            .child(Input::new(&self.label).id("vault-new-label"))
            .child(
                Input::new(&self.password)
                    .id("vault-new-password")
                    .mask_toggle(),
            )
            .when_some(self.error.clone(), |prompt, error| {
                prompt.child(form::error_text(error, cx))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("vault-add-cancel")
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("vault-add-confirm")
                            .primary()
                            .label("Add")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}

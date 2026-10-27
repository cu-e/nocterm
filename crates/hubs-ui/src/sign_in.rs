//! The dialog that signs in to a server: address and fingerprint, the trust decision, then a
//! password or the browser. The order of the steps belongs to [`SignInSession`]; this view
//! shows the current step and passes the user's choices on.

mod form;

use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable, Subscription,
    Window,
    component::{
        WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::Input,
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_hubs::{Hubs, SignInError, SignInEvent, SignInSession, SignInStep, TrustNote};
use nocterm_hubs_model::{HubId, OidcProvider};
use nocterm_session::Secret;

use crate::layout::{field, fingerprint_block, note, problem, title};
use crate::messages::sign_in_problem;
use form::SignInForm;

/// Starts signing in to a server not known yet, or again to `hub`, and shows the dialog. A hub
/// that is not known any more shows nothing.
pub(crate) fn start(hubs: &Entity<Hubs>, hub: Option<&HubId>, window: &mut Window, cx: &mut App) {
    let session = hubs.update(cx, |hubs, cx| match hub {
        Some(hub) => hubs.start_sign_in_for(hub, cx),
        None => Some(hubs.start_sign_in(cx)),
    });
    if let Some(session) = session {
        open(session, window, cx);
    }
}

/// Opens the dialog over `session`. Closing the dialog ends the sign-in: a browser sign-in
/// in flight is cancelled with the session.
pub(crate) fn open(session: Entity<SignInSession>, window: &mut Window, cx: &mut App) {
    let dialog = cx.new(|cx| SignInDialog::new(session, window, cx));
    let first = dialog.clone();
    window.open_dialog(cx, move |shell, _, _| {
        let submit = dialog.clone();
        shell
            .title("Sign in to a server")
            .w(px(480.))
            // Enter reaches the dialog as the confirm action. The step decides what it means
            // and the dialog closes itself when it is done.
            .on_ok(move |_, window, cx| {
                submit.update(cx, |dialog, cx| dialog.primary(window, cx));
                false
            })
            .footer(div())
            .child(dialog.clone())
    });
    first.update(cx, |dialog, cx| dialog.follow_step(window, cx));
}

struct SignInDialog {
    session: Entity<SignInSession>,
    form: SignInForm,
    focus: FocusHandle,
    /// The kind of step the focus was last placed for.
    placed: Option<std::mem::Discriminant<SignInStep>>,
    _subscriptions: Vec<Subscription>,
}

impl SignInDialog {
    fn new(session: Entity<SignInSession>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let form = SignInForm::new(window, cx);
        let subscriptions = vec![
            cx.observe_in(&session, window, |this, _, window, cx| {
                this.follow_step(window, cx);
            }),
            cx.subscribe(&session, |_, _, event, cx| {
                if let SignInEvent::OpenBrowser(url) = event {
                    cx.open_url(url);
                }
            }),
        ];
        Self {
            session,
            form,
            focus: cx.focus_handle(),
            placed: None,
            _subscriptions: subscriptions,
        }
    }

    /// Puts the cursor where the step asks for input when the step changes.
    fn follow_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let step = self.step(cx);
        let kind = std::mem::discriminant(&step);
        if self.placed != Some(kind) {
            self.placed = Some(kind);
            let target = match step {
                SignInStep::Address { .. } if self.session.read(cx).address_editable() => {
                    self.form.address().read(cx).focus_handle(cx)
                }
                SignInStep::Method { password: true, .. } => {
                    self.form.username().read(cx).focus_handle(cx)
                }
                SignInStep::Passphrase { .. } => self.form.passphrase().read(cx).focus_handle(cx),
                SignInStep::Recovery { .. } => self.form.recovery_key().read(cx).focus_handle(cx),
                _ => self.focus.clone(),
            };
            window.focus(&target, cx);
        }
        cx.notify();
    }

    fn step(&self, cx: &App) -> SignInStep {
        self.session.read(cx).step().clone()
    }

    /// What Enter does on this step. Never a choice that cannot be taken back: on the trust
    /// step it goes back.
    fn primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.step(cx) {
            SignInStep::Address { .. } if self.session.read(cx).address_editable() => {
                self.submit_address(cx);
            }
            SignInStep::Address { .. } | SignInStep::Failed(_) => self.close(window, cx),
            SignInStep::ConfirmFingerprint { .. } | SignInStep::Mismatch { .. } => {
                self.session.update(cx, |session, cx| session.back(cx));
            }
            SignInStep::Method { password: true, .. } => self.sign_in_with_password(window, cx),
            SignInStep::Finished(_) => self.close(window, cx),
            SignInStep::Passphrase { .. } => self.set_vault_passphrase(window, cx),
            SignInStep::Recovery { .. } => self.recover(window, cx),
            // Enter must not skip past the one showing of the recovery key.
            SignInStep::RecoveryKey { .. }
            | SignInStep::Method { .. }
            | SignInStep::Checking
            | SignInStep::Authenticating
            | SignInStep::Browser { .. }
            | SignInStep::Saving => {}
        }
    }

    fn close(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
    }

    fn submit_address(&mut self, cx: &mut Context<Self>) {
        let address = self.form.address_text(cx);
        let fingerprint = self.form.fingerprint_text(cx);
        self.session.update(cx, |session, cx| {
            session.submit_address(&address, &fingerprint, cx)
        });
    }

    fn sign_in_with_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let username = self.form.username_text(cx);
        let password = self.form.take_password(window, cx);
        self.session.update(cx, |session, cx| {
            session.sign_in_with_password(&username, password, cx);
        });
    }

    fn set_vault_passphrase(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let passphrase = self.form.take_passphrase(window, cx);
        self.session.update(cx, |session, cx| {
            session.set_vault_passphrase(passphrase, cx);
        });
    }

    fn recover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let recovery_key = self.form.take_recovery_key(window, cx);
        self.session.update(cx, |session, cx| {
            session.recover(recovery_key, cx);
        });
    }

    fn sign_in_with_provider(&mut self, provider: &str, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.sign_in_with_provider(provider, cx)
        });
    }

    fn render_step(&mut self, step: SignInStep, cx: &mut Context<Self>) -> impl IntoElement {
        match step {
            SignInStep::Address { problem } => self.render_address(problem.as_ref(), cx),
            SignInStep::Checking => self.render_busy("Checking the server…", true, cx),
            SignInStep::Authenticating => self.render_busy("Signing in…", false, cx),
            SignInStep::Saving => self.render_busy("Saving the server…", false, cx),
            SignInStep::ConfirmFingerprint {
                address,
                fingerprint,
            } => self.render_confirm(&address.to_string(), &fingerprint, cx),
            SignInStep::Mismatch { typed, presented } => {
                self.render_mismatch(&typed, &presented, cx)
            }
            SignInStep::Method {
                hub_name,
                password,
                providers,
                note,
                problem,
            } => self.render_method(&hub_name, password, &providers, note, problem.as_ref(), cx),
            SignInStep::Browser { provider, url } => self.render_browser(&provider, url, cx),
            SignInStep::RecoveryKey { key } => self.render_recovery_key(&key, cx),
            SignInStep::Passphrase { problem } => self.render_passphrase(problem.as_ref(), cx),
            SignInStep::Recovery { problem } => self.render_recovery(problem.as_ref(), cx),
            SignInStep::Finished(_) => self.render_finished(cx),
            SignInStep::Failed(error) => self.render_failed(&error, cx),
        }
    }

    fn render_address(
        &mut self,
        failure: Option<&SignInError>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let editable = self.session.read(cx).address_editable();
        v_flex()
            .gap_3()
            .when(editable, |step| {
                step.child(field("Server", Input::new(self.form.address())))
                    .child(field(
                        "Certificate fingerprint",
                        Input::new(self.form.fingerprint()),
                    ))
                    .child(note(
                        "Leave the fingerprint empty to see the certificate first. A server with a certificate from a public authority needs none.",
                        cx,
                    ))
            })
            .when_some(failure, |step, error| {
                step.child(problem(sign_in_problem(error), cx))
            })
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("sign-in-cancel")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.close(window, cx);
                            })),
                    )
                    .when(editable, |buttons| {
                        buttons.child(
                            Button::new("sign-in-continue")
                                .primary()
                                .label("Continue")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.submit_address(cx);
                                })),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_busy(
        &mut self,
        what: &'static str,
        cancels_dialog: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        v_flex()
            .gap_3()
            .child(note(what, cx))
            .child(
                h_flex()
                    .justify_end()
                    .child(
                        Button::new("sign-in-stop")
                            .label("Cancel")
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                if cancels_dialog {
                                    this.close(window, cx);
                                } else {
                                    this.session.update(cx, |session, cx| session.back(cx));
                                }
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_confirm(
        &mut self,
        address: &str,
        fingerprint: &nocterm_hubs_model::Fingerprint,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        v_flex()
            .gap_3()
            .child(title(format!("Trust {address}?"), cx))
            .child(note(
                "This server's certificate is not from a public authority. Compare its fingerprint with the one shown by the server, then trust and pin it.",
                cx,
            ))
            .child(fingerprint_block("Fingerprint", fingerprint, cx))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("sign-in-back")
                            .primary()
                            .label("Back")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.session.update(cx, |session, cx| session.back(cx));
                            })),
                    )
                    .child(
                        Button::new("sign-in-trust")
                            .label("Trust and pin this certificate")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.session
                                    .update(cx, |session, cx| session.confirm_fingerprint(cx));
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_mismatch(
        &mut self,
        typed: &nocterm_hubs_model::Fingerprint,
        presented: &nocterm_hubs_model::Fingerprint,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        v_flex()
            .gap_3()
            .child(problem(
                "The server's certificate is not the one you typed. Do not continue unless you know why it changed."
                    .to_owned(),
                cx,
            ))
            .child(fingerprint_block("You typed", typed, cx))
            .child(fingerprint_block("The server presented", presented, cx))
            .child(
                h_flex().justify_end().child(
                    Button::new("sign-in-back")
                        .primary()
                        .label("Back")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.session.update(cx, |session, cx| session.back(cx));
                        })),
                ),
            )
            .into_any_element()
    }

    fn render_method(
        &mut self,
        hub_name: &str,
        password: bool,
        providers: &[OidcProvider],
        trust: TrustNote,
        failure: Option<&SignInError>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let editable = self.session.read(cx).address_editable();
        let trust = match trust {
            TrustNote::Pinned => "The certificate matches the fingerprint.",
            TrustNote::PublicCa => "The certificate is vouched for by a public authority.",
            TrustNote::Confirmed => "The certificate was pinned after you confirmed it.",
        };
        v_flex()
            .gap_3()
            .child(title(format!("Sign in to {hub_name}"), cx))
            .child(note(trust, cx))
            .when(password, |step| {
                step.child(field("User name", Input::new(self.form.username())))
                    .child(field("Password", Input::new(self.form.password())))
            })
            .when_some(failure, |step, error| {
                step.child(problem(sign_in_problem(error), cx))
            })
            .child(v_flex().gap_2().children(providers.iter().map(|provider| {
                let id = provider.id.clone();
                Button::new(provider_key(&provider.id))
                    .w_full()
                    .label(format!("Continue with {}", provider.display_name))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.sign_in_with_provider(&id, cx);
                    }))
            })))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("sign-in-cancel")
                            .label(if editable { "Back" } else { "Cancel" })
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                if editable {
                                    this.session.update(cx, |session, cx| session.back(cx));
                                } else {
                                    this.close(window, cx);
                                }
                            })),
                    )
                    .when(password, |buttons| {
                        buttons.child(
                            Button::new("sign-in-password")
                                .primary()
                                .label("Sign in")
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.sign_in_with_password(window, cx);
                                })),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_browser(
        &mut self,
        provider: &str,
        url: Option<String>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        v_flex()
            .gap_3()
            .child(note(
                format!("Finish signing in with {provider} in your browser. This window continues by itself."),
                cx,
            ))
            .when_some(url.clone(), |step, url| step.child(note(url, cx)))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .when_some(url, |buttons, url| {
                        buttons.child(Button::new("sign-in-copy-link").label("Copy link").on_click(
                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                    url.clone(),
                                ));
                            },
                        ))
                    })
                    .child(
                        Button::new("sign-in-stop")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.session.update(cx, |session, cx| session.back(cx));
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_recovery_key(
        &mut self,
        key: &Secret,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let text = key.expose().to_owned();
        v_flex()
            .gap_3()
            .child(title("Write down your recovery key".to_owned(), cx))
            .child(note(
                "Your account keys were created. This key is the only way to open them if you lose your password, and it is not shown again or kept on this device.",
                cx,
            ))
            .child(note(text.clone(), cx))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("sign-in-copy-key").label("Copy key").on_click(
                        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                text.clone(),
                            ));
                        },
                    ))
                    .child(
                        Button::new("sign-in-key-saved")
                            .primary()
                            .label("I have saved it")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.session.update(cx, |session, cx| {
                                    session.acknowledge_recovery_key(cx);
                                });
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_passphrase(
        &mut self,
        failure: Option<&SignInError>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        v_flex()
            .gap_3()
            .child(title("Vault passphrase".to_owned(), cx))
            .child(note(
                "Signing in through a provider does not open your account keys. Type the passphrase that protects them; the first time, this sets it.",
                cx,
            ))
            .child(field("Passphrase", Input::new(self.form.passphrase())))
            .when_some(failure, |step, error| {
                step.child(problem(sign_in_problem(error), cx))
            })
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("sign-in-passphrase-back")
                            .label("Back")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.session.update(cx, |session, cx| session.back(cx));
                            })),
                    )
                    .child(
                        Button::new("sign-in-passphrase")
                            .primary()
                            .label("Continue")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.set_vault_passphrase(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_recovery(
        &mut self,
        failure: Option<&SignInError>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        v_flex()
            .gap_3()
            .child(title("Enter your recovery key".to_owned(), cx))
            .child(note(
                "This password does not open your account keys; it was probably reset. The recovery key you saved when the account was created opens them, and this password opens them from now on.",
                cx,
            ))
            .child(field("Recovery key", Input::new(self.form.recovery_key())))
            .when_some(failure, |step, error| {
                step.child(problem(sign_in_problem(error), cx))
            })
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("sign-in-recovery-back")
                            .label("Back")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.session.update(cx, |session, cx| session.back(cx));
                            })),
                    )
                    .child(
                        Button::new("sign-in-recovery")
                            .primary()
                            .label("Recover")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.recover(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_finished(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        v_flex()
            .gap_3()
            .child(title("Signed in".to_owned(), cx))
            .child(note(
                "The server is saved. Its sessions are in the Sessions panel.",
                cx,
            ))
            .child(
                h_flex().justify_end().child(
                    Button::new("sign-in-done")
                        .primary()
                        .label("Done")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.close(window, cx);
                        })),
                ),
            )
            .into_any_element()
    }

    fn render_failed(
        &mut self,
        error: &SignInError,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        v_flex()
            .gap_3()
            .child(problem(sign_in_problem(error), cx))
            .child(
                h_flex().justify_end().child(
                    Button::new("sign-in-close")
                        .primary()
                        .label("Close")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.close(window, cx);
                        })),
                ),
            )
            .into_any_element()
    }
}

/// A stable element id part for a provider.
fn provider_key(id: &str) -> gpui_kit::SharedString {
    format!("sign-in-provider-{id}").into()
}

impl Focusable for SignInDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SignInDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let step = self.step(cx);
        v_flex()
            .id("sign-in-dialog")
            .track_focus(&self.focus)
            .w_full()
            .child(self.render_step(step, cx))
    }
}

#[cfg(test)]
#[path = "sign_in_tests.rs"]
mod tests;

//! Unlocking and locking the vault from anywhere: the command palette, or a
//! sign-in prompt that could use a saved secret.
use futures::{FutureExt as _, StreamExt as _};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, Render, SharedString, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Icon, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_session::Secret;
use nocterm_ui::IconName;
use nocterm_vault::{DeviceAvailability, DeviceUnlockError, VaultError, VaultService};
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
        let submit = prompt.clone();
        dialog
            .title("Unlock the vault")
            .w(px(420.))
            // Enter reaches the dialog as Confirm, which would close it before
            // the password is submitted. The prompt closes itself once unlocked.
            .on_ok(move |_, window, cx| {
                submit.update(cx, |prompt, cx| prompt.submit(window, cx));
                false
            })
            .child(prompt.clone())
    });
    window.focus(&focus, cx);
}

pub(crate) struct UnlockPrompt {
    service: Arc<VaultService>,
    password: Entity<InputState>,
    busy: bool,
    pub(crate) error: Option<SharedString>,
    /// The device unlock label ("Fingerprint") once it is known to be enabled.
    pub(crate) device: Option<SharedString>,
    /// A device authentication is waiting on the vault worker.
    pub(crate) scanning: bool,
    /// Identifies the latest scan, so a cancelled one cannot report late.
    scan: u64,
    pub(crate) attempts_remaining: Option<u8>,
    pub(crate) device_locked: bool,
}

impl UnlockPrompt {
    pub(crate) fn new(
        service: Arc<VaultService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Master password")
        });
        let probe = service.probe_device_unlock();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(capability) = probe.await else {
                return;
            };
            if capability.availability == DeviceAvailability::LockedOut {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.device = Some(capability.label.into());
                    this.device_locked = true;
                    this.attempts_remaining = Some(0);
                    this.error = Some(capability.detail.into());
                    cx.notify();
                });
                return;
            }
            if !capability.armed || capability.availability != DeviceAvailability::Available {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.device = Some(capability.label.into());
                this.unlock_with_device(window, cx);
            });
        })
        .detach();
        Self {
            service,
            password,
            busy: false,
            error: None,
            device: None,
            scanning: false,
            scan: 0,
            attempts_remaining: None,
            device_locked: false,
        }
    }

    /// Starts device authentication; typing the password stays possible and
    /// cancels it on submit.
    fn unlock_with_device(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.scanning || self.device_locked || self.service.is_unlocked() {
            return;
        }
        self.scanning = true;
        self.scan += 1;
        let scan = self.scan;
        self.error = None;
        self.attempts_remaining = None;
        let (future, mut progress) = self.service.unlock_with_device_progress();
        cx.spawn_in(window, async move |this, cx| {
            let future = future.fuse();
            futures::pin_mut!(future);
            let result = loop {
                futures::select_biased! {
                    result = future => break result,
                    remaining = progress.next().fuse() => {
                        let Some(remaining) = remaining else { break future.await; };
                        let _ = this.update_in(cx, |this, _, cx| {
                            this.scan_progress(scan, remaining);
                            cx.notify();
                        });
                    }
                }
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.scanning || this.scan != scan {
                    return;
                }
                this.scanning = false;
                match result {
                    Ok(()) => {
                        window.close_dialog(cx);
                        window.refresh();
                    }
                    Err(
                        VaultError::Cancelled | VaultError::Device(DeviceUnlockError::Cancelled),
                    ) => {}
                    Err(VaultError::Device(DeviceUnlockError::Locked)) => {
                        this.device_locked = true;
                        this.attempts_remaining = Some(0);
                        this.error = Some(DeviceUnlockError::Locked.to_string().into());
                    }
                    Err(error) => this.error = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn scan_progress(&mut self, scan: u64, remaining: u8) {
        if self.scanning && self.scan == scan {
            self.attempts_remaining = Some(remaining);
        }
    }
    fn cancel_device(&mut self) {
        if std::mem::take(&mut self.scanning) {
            // Locking advances the vault epoch, which cancels the native prompt.
            self.service.lock();
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.password.read(cx).value().to_string();
        if self.busy || typed.is_empty() {
            return;
        }
        self.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.cancel_device();
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

impl Drop for UnlockPrompt {
    fn drop(&mut self) {
        // Closing the dialog must not leave a fingerprint scan running.
        self.cancel_device();
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
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.password).mask_toggle()),
                    )
                    .when_some(self.device.clone(), |row, label| {
                        row.child(
                            Button::new("vault-unlock-device")
                                .outline()
                                .icon(Icon::new(IconName::FingerprintPattern).size_5())
                                .accessibility_label(format!("Unlock with {label}"))
                                .tooltip(format!("Unlock with {label}"))
                                .when(self.scanning, |button| {
                                    button.text_color(theme.primary).border_color(theme.primary)
                                })
                                .disabled(self.busy || self.device_locked)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.unlock_with_device(window, cx)
                                })),
                        )
                    }),
            )
            .when(self.scanning, |prompt| {
                let label = self.device.clone().unwrap_or_default();
                let detail = match self.attempts_remaining {
                    Some(0) => format!("{label}: no attempts remaining. Unlock with the master password."),
                    Some(1) => format!("{label}: 1 attempt remaining. Touch the sensor, or type the master password."),
                    Some(remaining) => format!("{label}: {remaining} attempts remaining. Touch the sensor, or type the master password."),
                    None => format!("{label}: touch the sensor, or type the master password."),
                };
                prompt.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(detail),
                )
            })
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

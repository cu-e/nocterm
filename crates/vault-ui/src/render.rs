//! The Vault page's tabs.
use gpui_kit::{
    AnyElement, Context, Window,
    base::TestSupportExt as _,
    component::{
        Disableable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        switch::Switch,
        tab::{Tab as TabButton, TabBar},
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_ui::{IconName, form};
use nocterm_vault::DeviceAvailability;

use crate::view::{Tab, VaultView};

impl Render for VaultView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = Tab::ALL
            .iter()
            .position(|tab| *tab == self.tab)
            .unwrap_or(0);
        let view = cx.weak_entity();
        let tabs = TabBar::new("vault-tabs")
            .underline()
            .selected_index(selected)
            .children(Tab::ALL.map(|tab| TabButton::new().label(tab.title())))
            .on_click(move |index, window, cx| {
                let index = *index;
                let _ = view.update(cx, |view, cx| view.select_tab(Tab::ALL[index], window, cx));
            });
        let body = match self.tab {
            Tab::Overview => self.overview(cx),
            Tab::Credentials => self.credentials.clone().into_any_element(),
            Tab::Security => self.security(cx),
            Tab::Options => self.options(cx),
        };
        v_flex()
            .w_full()
            .track_focus(&self.focus)
            .child(form::page_header(
                "Vault",
                Some("Passwords and key passphrases you chose to remember, encrypted with a master password.".into()),
                cx,
            ))
            .child(tabs)
            .child(body)
            .when_some(self.message.clone(), |page, (message, error)| {
                page.child(div().id("vault-message").role(gpui_kit::Role::Alert).aria_label(message.clone()).test_support().child(if error {
                    form::error_text(message,cx)
                }else{form::note(message,cx)}))
            })
            .when(self.busy, |page| {
                page.child(
                    h_flex()
                        .gap_2()
                        .pt_2()
                        .child(form::note("Working…", cx))
                        .child(
                            Button::new("vault-cancel-operation")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, window, cx| this.lock(window, cx))),
                        ),
                )
            })
    }
}

impl VaultView {
    fn overview(&self, cx: &mut Context<Self>) -> AnyElement {
        let unlocked = self.service.is_unlocked();
        let exists = self.service.exists();
        if let (false, false, Some(remedy)) = (unlocked, exists, self.keys_missing.clone()) {
            return form::section(
                "Status",
                vec![form::row(
                    "Keys not on this computer",
                    "This server account's keys are not on this computer yet. Sign in again to open them.",
                    Button::new("vault-keys-missing")
                        .small()
                        .primary()
                        .label("Sign in again")
                        .on_click(move |_, window, cx| {
                            let remedy = remedy.clone();
                            window.defer(cx, move |window, cx| remedy(window, cx));
                        }),
                    cx,
                )],
                cx,
            );
        }
        let device = self
            .device
            .clone()
            .filter(|device| device.armed && device.availability == DeviceAvailability::Available);
        let (state, detail) = if unlocked {
            (
                "Unlocked",
                "Saved credentials are used to sign in. Lock the vault to require the master password again.",
            )
        } else if exists {
            (
                "Locked",
                "Saved credentials and their names stay encrypted until you unlock.",
            )
        } else {
            (
                "Not set up",
                "Create a vault to remember passwords and passphrases. Forgetting the master password makes them unrecoverable.",
            )
        };
        let mut rows = vec![form::row(
            state,
            detail,
            Button::new("vault-lock")
                .small()
                .ghost()
                .icon(IconName::Lock)
                .label("Lock")
                .disabled(!unlocked && !self.busy)
                .on_click(cx.listener(|this, _, window, cx| this.lock(window, cx))),
            cx,
        )];
        if !unlocked {
            rows.push(self.password_form(exists, cx));
        }
        if let (false, Some(device)) = (unlocked, device) {
            rows.push(form::row(
                format!("Unlock with {}", device.label),
                device.detail.clone(),
                Button::new("vault-device-unlock")
                    .small()
                    .label("Unlock")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.device_operation(this.service.unlock_with_device(), cx)
                    })),
                cx,
            ));
        }
        form::section("Status", rows, cx)
    }

    fn password_form(&self, exists: bool, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .w_full()
            .py_3()
            .gap_2()
            .child(div().text_sm().font_medium().child(if exists {
                "Master password"
            } else {
                "Choose a master password"
            }))
            .child(
                div()
                    .id("vault-password")
                    .test_support()
                    .child(self.form.password_input()),
            )
            .when(!exists, |form| {
                form.child(
                    div()
                        .id("vault-confirm")
                        .test_support()
                        .child(self.form.confirm_input()),
                )
            })
            .child(
                h_flex().child(
                    Button::new("vault-submit")
                        .small()
                        .primary()
                        .label(if exists { "Unlock" } else { "Create vault" })
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                ),
            )
            .into_any_element()
    }

    fn security(&self, cx: &mut Context<Self>) -> AnyElement {
        let unlocked = self.service.is_unlocked();
        let password = if unlocked {
            v_flex()
                .w_full()
                .py_3()
                .gap_2()
                .child(
                    div()
                        .id("vault-password")
                        .test_support()
                        .child(self.form.password_input()),
                )
                .child(
                    div()
                        .id("vault-confirm")
                        .test_support()
                        .child(self.form.confirm_input()),
                )
                .child(
                    h_flex().child(
                        Button::new("vault-submit")
                            .small()
                            .label("Change master password")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
                )
                .into_any_element()
        } else {
            form::note("Unlock the vault to change its master password.", cx)
        };
        let device = match self.device.clone() {
            None => vec![form::note("Checking for fingerprint or system unlock…", cx)],
            Some(device) => vec![form::row(
                device.label.clone(),
                device.detail.clone(),
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("vault-device-enable")
                            .small()
                            .label("Turn on")
                            .disabled(
                                self.busy
                                    || !unlocked
                                    || device.armed
                                    || device.availability != DeviceAvailability::Available,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.device_operation(this.service.enable_device_unlock(), cx)
                            })),
                    )
                    .child(
                        Button::new("vault-device-disable")
                            .small()
                            .ghost()
                            .label("Turn off")
                            .disabled(self.busy || !device.enabled)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.device_operation(this.service.disable_device_unlock(), cx)
                            })),
                    )
                    .child(
                        Button::new("vault-device-refresh")
                            .small()
                            .ghost()
                            .icon(IconName::RefreshCw)
                            .tooltip("Check again")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_device(cx))),
                    ),
                cx,
            )],
        };
        v_flex()
            .w_full()
            .child(form::section("Master password", [password], cx))
            .child(form::section("Device unlock", device, cx))
            .into_any_element()
    }

    fn options(&self, cx: &mut Context<Self>) -> AnyElement {
        let lock = v_flex()
            .child(form::row(
                "Lock automatically",
                "Minutes without using the vault before it locks, 1–1440.",
                div()
                    .id("vault-auto-lock")
                    .test_support()
                    .w(rems(6.))
                    .child(self.form.auto_lock_input()),
                cx,
            ))
            .when_some(self.form.auto_lock_error(), |row, error| {
                row.child(form::error_text(error, cx))
            })
            .into_any_element();
        let startup = form::row(
            "Ask for the master password at startup",
            "Open this page when nocterm starts, so saved credentials are ready.",
            Switch::new("vault-prompt-on-startup")
                .checked(
                    self.settings
                        .setting::<crate::VaultSettings>(cx)
                        .prompt_on_startup,
                )
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    let checked = *checked;
                    this.settings
                        .update::<crate::VaultSettings>(cx, move |settings| {
                            settings.prompt_on_startup = checked
                        })
                        .detach();
                    cx.notify();
                })),
            cx,
        );
        form::section("Behavior", [lock, startup], cx)
    }
}

//! The Vault page's tabs.
use gpui_kit::{
    AnyElement, ClipboardItem, Context, Window,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::Input,
        switch::Switch,
        tab::{Tab as TabButton, TabBar},
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_ui::{IconName, SettingsExt as _, form};
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
            Tab::Credentials => self.credentials(cx),
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
                page.child(if error {
                    form::error_text(message, cx)
                } else {
                    form::note(message, cx)
                })
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
            .child(Input::new(&self.password))
            .when(!exists, |form| form.child(Input::new(&self.confirm)))
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

    fn credentials(&self, cx: &mut Context<Self>) -> AnyElement {
        if !self.service.is_unlocked() {
            return form::section(
                "Saved credentials",
                [form::note(
                    "Unlock the vault on the Overview tab to see what it holds.",
                    cx,
                )],
                cx,
            );
        }
        let mut rows: Vec<AnyElement> = self
            .records
            .iter()
            .enumerate()
            .map(|(index, record)| {
                let id = record.id;
                h_flex()
                    .w_full()
                    .py_3()
                    .gap_2()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().child(record.label.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(id.to_string()),
                            ),
                    )
                    .child(
                        Button::new(("vault-copy-id", index))
                            .small()
                            .ghost()
                            .icon(IconName::Copy)
                            .tooltip("Copy ID")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(id.to_string()))
                            }),
                    )
                    .child(
                        Button::new(("vault-delete", index))
                            .small()
                            .ghost()
                            .icon(IconName::Trash)
                            .tooltip("Delete")
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.operation(this.service.delete(id), cx)
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        if rows.is_empty() {
            rows.push(form::note(
                "Nothing saved yet. Choose Remember when you sign in to a server.",
                cx,
            ));
        }
        v_flex()
            .w_full()
            .child(form::section("Saved credentials", rows, cx))
            .child(
                h_flex().pt_2().child(
                    Button::new("vault-refresh")
                        .small()
                        .ghost()
                        .icon(IconName::RefreshCw)
                        .label("Refresh")
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
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
                .child(Input::new(&self.password))
                .child(Input::new(&self.confirm))
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
                div().w(rems(6.)).child(Input::new(&self.auto_lock).small()),
                cx,
            ))
            .when_some(self.auto_lock_error.clone(), |row, error| {
                row.child(form::error_text(error, cx))
            })
            .into_any_element();
        let startup = form::row(
            "Ask for the master password at startup",
            "Open this page when nocterm starts, so saved credentials are ready.",
            Switch::new("vault-prompt-on-startup")
                .checked(cx.setting::<crate::VaultSettings>().prompt_on_startup)
                .on_click(cx.listener(|_, checked: &bool, _, cx| {
                    let checked = *checked;
                    cx.update_setting::<crate::VaultSettings>(move |settings| {
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

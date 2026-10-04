//! The Vault page's state and the operations it starts.
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, SharedString, Subscription, Task, Window,
    component::input::{InputEvent, InputState},
    prelude::*,
};
use nocterm_session::Secret;
use nocterm_ui::{ActiveSettings as _, SettingsStore, edit_settings};
use nocterm_vault::{CredentialInfo, DeviceCapability, VaultFuture, VaultService};
use nocterm_workspace::SettingsPage;
use std::{sync::Arc, time::Duration};

use crate::Service;

/// The page's tabs, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Overview,
    Credentials,
    Security,
    Options,
}

impl Tab {
    pub(crate) const ALL: [Tab; 4] = [Tab::Overview, Tab::Credentials, Tab::Security, Tab::Options];
    pub(crate) fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Credentials => "Credentials",
            Tab::Security => "Security",
            Tab::Options => "Options",
        }
    }
}

pub struct VaultView {
    pub(crate) tab: Tab,
    pub(crate) service: Arc<VaultService>,
    pub(crate) password: Entity<InputState>,
    pub(crate) confirm: Entity<InputState>,
    pub(crate) focus: FocusHandle,
    pub(crate) busy: bool,
    pub(crate) device: Option<DeviceCapability>,
    pub(crate) device_busy: bool,
    pub(crate) message: Option<(SharedString, bool)>,
    pub(crate) records: Vec<CredentialInfo>,
    pub(crate) unlocked: bool,
    pub(crate) generation: u64,
    /// Minutes before the vault locks itself, saved when the field is left.
    pub(crate) auto_lock: Entity<InputState>,
    pub(crate) auto_lock_error: Option<SharedString>,
    _watch: Task<()>,
    _subscriptions: Vec<Subscription>,
}
impl VaultView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Master password")
        });
        let confirm = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Repeat master password")
        });
        let auto_lock = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(cx.settings().vault.auto_lock_minutes.to_string())
        });
        let subscriptions = vec![
            cx.subscribe_in(&password, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) && !this.service.is_unlocked() {
                    this.submit(window, cx);
                }
            }),
            cx.subscribe_in(&auto_lock, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.save_auto_lock(cx);
                }
            }),
            cx.observe_global_in::<SettingsStore>(window, |this, window, cx| {
                let minutes = cx.settings().vault.auto_lock_minutes.to_string();
                let focused = this.auto_lock.read(cx).focus_handle(cx).is_focused(window);
                if !focused && this.auto_lock.read(cx).value() != minutes.as_str() {
                    this.auto_lock
                        .update(cx, |input, cx| input.set_value(minutes, window, cx));
                }
                cx.notify();
            }),
        ];
        let mut this = Self {
            tab: Tab::Overview,
            auto_lock,
            auto_lock_error: None,
            service: cx.global::<Service>().0.clone(),
            focus: cx.focus_handle(),
            password,
            confirm,
            busy: false,
            device: None,
            device_busy: false,
            message: None,
            records: Vec::new(),
            unlocked: false,
            generation: 0,
            _subscriptions: subscriptions,
            _watch: cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(250))
                        .await;
                    if !matches!(
                        cx.update(|window, app| this.update(app, |this, cx| {
                            let unlocked = this.service.is_unlocked();
                            if unlocked != this.unlocked {
                                this.unlocked = unlocked;
                                if !unlocked {
                                    this.records.clear();
                                    this.generation = this.generation.wrapping_add(1);
                                    this.busy = false;
                                    this.device_busy = false;
                                    this.password
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    this.confirm
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                } else {
                                    this.refresh(cx);
                                }
                                cx.notify();
                            }
                        })),
                        Ok(Ok(()))
                    ) {
                        break;
                    }
                }
            }),
        };
        this.refresh_device(cx);
        if this.service.is_unlocked() {
            this.unlocked = true;
            this.refresh(cx);
        }
        this
    }
    pub(crate) fn refresh_device(&mut self, cx: &mut Context<Self>) {
        let future = self.service.probe_device_unlock();
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if generation == this.generation {
                    if let Ok(capability) = result {
                        this.device = Some(capability);
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        let future = self.service.list();
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation || !this.service.is_unlocked() {
                    return;
                }
                match result {
                    Ok(records) => this.records = records,
                    Err(error) => this.message = Some((error.to_string().into(), true)),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let password = Secret::new(self.password.read(cx).value().to_string());
        if (!self.service.exists() || self.service.is_unlocked())
            && password.expose() != self.confirm.read(cx).value().as_ref()
        {
            self.message = Some(("The master passwords do not match.".into(), true));
            cx.notify();
            return;
        }
        self.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.confirm
            .update(cx, |input, cx| input.set_value("", window, cx));
        let future = if self.service.is_unlocked() {
            self.service.change_password(password)
        } else if self.service.exists() {
            self.service.unlock(password)
        } else {
            self.service.create(password)
        };
        self.operation(future, cx);
    }
    pub(crate) fn device_operation(&mut self, future: VaultFuture<()>, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.device_busy = true;
        self.operation(future, cx);
    }
    pub(crate) fn operation(&mut self, future: VaultFuture<()>, cx: &mut Context<Self>) {
        self.busy = true;
        self.message = None;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.busy = false;
                this.device_busy = false;
                this.refresh_device(cx);
                match result {
                    Ok(()) => {
                        this.unlocked = this.service.is_unlocked();
                        this.message = Some(("Vault updated.".into(), false));
                        if this.unlocked {
                            this.refresh(cx);
                        }
                    }
                    Err(error) => this.message = Some((error.to_string().into(), true)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.service.lock();
        self.device_busy = false;
        self.unlocked = false;
        self.records.clear();
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        self.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.confirm
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.message = Some(("Vault locked.".into(), false));
        cx.notify();
    }
}
impl VaultView {
    /// Saves the automatic lock delay if it is a whole number of minutes in range.
    pub(crate) fn save_auto_lock(&mut self, cx: &mut Context<Self>) {
        let text = self.auto_lock.read(cx).value().trim().to_owned();
        match text.parse::<u32>() {
            Ok(minutes) if (1..=1440).contains(&minutes) => {
                self.auto_lock_error = None;
                if minutes != cx.settings().vault.auto_lock_minutes {
                    edit_settings(cx, move |settings| {
                        settings.vault.auto_lock_minutes = minutes
                    })
                    .detach();
                }
            }
            _ => {
                self.auto_lock_error = Some("Enter a number of minutes from 1 to 1440.".into());
            }
        }
        cx.notify();
    }
    pub(crate) fn select_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        // Master passwords never wait in a hidden tab.
        self.clear_passwords(window, cx);
        self.message = None;
        self.tab = tab;
        cx.notify();
    }
    fn clear_passwords(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.confirm
            .update(cx, |input, cx| input.set_value("", window, cx));
    }
}

impl Focusable for VaultView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.busy {
            self.focus.clone()
        } else {
            self.password.read(cx).focus_handle(cx)
        }
    }
}
impl SettingsPage for VaultView {
    fn on_deactivate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.device_busy {
            self.lock(window, cx);
        }
        self.clear_passwords(window, cx);
    }
    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_deactivate(window, cx);
        self.generation = self.generation.wrapping_add(1);
        self.records.clear();
    }
}

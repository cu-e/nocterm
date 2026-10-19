//! The Vault page's state and the operations it starts.
use gpui_kit::{App, Context, FocusHandle, Focusable, SharedString, Subscription, Window};
use nocterm_ui::SettingsPage;
use nocterm_vault::{CredentialInfo, DeviceCapability, VaultAccess, VaultFuture};
use std::sync::Arc;

mod form;
use form::VaultForm;
#[path = "render.rs"]
mod render;

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
    fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Credentials => "Credentials",
            Tab::Security => "Security",
            Tab::Options => "Options",
        }
    }
}

struct ViewBindings {
    _subscriptions: Vec<Subscription>,
    _status: gpui_kit::Task<()>,
}

pub struct VaultView {
    tab: Tab,
    service: Arc<dyn VaultAccess>,
    form: VaultForm,
    settings: nocterm_store::SettingsHandle,
    focus: FocusHandle,
    busy: bool,
    device: Option<DeviceCapability>,
    device_busy: bool,
    message: Option<(SharedString, bool)>,
    records: Vec<CredentialInfo>,
    unlocked: bool,
    generation: u64,
    _bindings: ViewBindings,
}
impl VaultView {
    pub(crate) fn new(
        service: Arc<dyn VaultAccess>,
        settings: nocterm_store::SettingsHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let form = VaultForm::new(
            settings
                .setting::<crate::VaultSettings>(cx)
                .auto_lock_minutes,
            window,
            cx,
        );
        let mut subscriptions = form.subscriptions(window, cx);
        let weak = cx.weak_entity();
        let window_handle = window.window_handle();
        let policy = settings.clone();
        subscriptions.push(settings.observe(cx, move |cx| {
            let minutes = policy.setting::<crate::VaultSettings>(cx).auto_lock_minutes;
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.form.sync_auto_lock(minutes, window, cx);
                    cx.notify();
                });
            });
        }));
        let status_service = service.clone();
        let mut seen = service.revision();
        let watch = cx.spawn_in(window, async move |this, cx| {
            loop {
                seen = status_service.changed(seen).await;
                if this
                    .update_in(cx, |this, window, cx| this.status_changed(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut this = Self {
            tab: Tab::Overview,
            form,
            settings,
            service,
            focus: cx.focus_handle(),
            busy: false,
            device: None,
            device_busy: false,
            message: None,
            records: Vec::new(),
            unlocked: false,
            generation: 0,
            _bindings: ViewBindings {
                _subscriptions: subscriptions,
                _status: watch,
            },
        };
        this.refresh_device(cx);
        if this.service.is_unlocked() {
            this.unlocked = true;
            this.refresh(cx);
        }
        this
    }
    /// Follows the vault locking or unlocking, by this page or any other path.
    fn status_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let unlocked = self.service.is_unlocked();
        if unlocked == self.unlocked {
            return;
        }
        self.unlocked = unlocked;
        if unlocked {
            self.refresh(cx);
        } else {
            self.records.clear();
            self.generation = self.generation.wrapping_add(1);
            self.busy = false;
            self.device_busy = false;
            self.form.clear_passwords(window, cx);
        }
        cx.notify();
    }
    fn refresh_device(&mut self, cx: &mut Context<Self>) {
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
    fn refresh(&mut self, cx: &mut Context<Self>) {
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
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let password = self.form.password(cx);
        if (!self.service.exists() || self.service.is_unlocked())
            && !self.form.confirm_matches(&password, cx)
        {
            self.message = Some(("The master passwords do not match.".into(), true));
            cx.notify();
            return;
        }
        self.form.clear_passwords(window, cx);
        let future = if self.service.is_unlocked() {
            self.service.change_password(password)
        } else if self.service.exists() {
            self.service.unlock(password)
        } else {
            self.service.create(password)
        };
        self.operation(future, cx);
    }
    fn device_operation(&mut self, future: VaultFuture<()>, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.device_busy = true;
        self.operation(future, cx);
    }
    fn operation(&mut self, future: VaultFuture<()>, cx: &mut Context<Self>) {
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
    fn lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.service.lock();
        self.device_busy = false;
        self.unlocked = false;
        self.records.clear();
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        self.form.clear_passwords(window, cx);
        self.message = Some(("Vault locked.".into(), false));
        cx.notify();
    }
}
impl VaultView {
    /// Saves the automatic lock delay if it is a whole number of minutes in range.
    fn save_auto_lock(&mut self, cx: &mut Context<Self>) {
        if let Some(minutes) = self.form.auto_lock_minutes(cx)
            && minutes
                != self
                    .settings
                    .setting::<crate::VaultSettings>(cx)
                    .auto_lock_minutes
        {
            self.settings
                .update::<crate::VaultSettings>(cx, move |settings| {
                    settings.auto_lock_minutes = minutes
                })
                .detach();
        }
        cx.notify();
    }
    fn select_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        // Master passwords never wait in a hidden tab.
        self.form.clear_passwords(window, cx);
        self.message = None;
        self.tab = tab;
        cx.notify();
    }
}

impl Focusable for VaultView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.busy {
            self.focus.clone()
        } else {
            self.form.focus_handle(cx)
        }
    }
}
impl SettingsPage for VaultView {
    fn on_deactivate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.device_busy {
            self.lock(window, cx);
        }
        self.form.clear_passwords(window, cx);
    }
    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_deactivate(window, cx);
        self.generation = self.generation.wrapping_add(1);
        self.records.clear();
    }
}

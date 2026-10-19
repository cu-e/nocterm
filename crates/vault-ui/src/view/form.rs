//! Actual masked native controls and their local confirmation/auto-lock transitions.
use super::VaultView;
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, SharedString, Subscription,
    Window,
    component::{
        Sizable as _,
        input::{Input, InputEvent, InputState},
    },
};
use nocterm_session::Secret;

pub(super) struct VaultForm {
    password: Entity<InputState>,
    confirm: Entity<InputState>,
    auto_lock: Entity<InputState>,
    auto_lock_error: Option<SharedString>,
}

impl VaultForm {
    pub(super) fn new(minutes: u32, window: &mut Window, cx: &mut Context<VaultView>) -> Self {
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
        let auto_lock = cx.new(|cx| InputState::new(window, cx).default_value(minutes.to_string()));
        Self {
            password,
            confirm,
            auto_lock,
            auto_lock_error: None,
        }
    }
    pub(super) fn subscriptions(
        &self,
        window: &mut Window,
        cx: &mut Context<VaultView>,
    ) -> Vec<Subscription> {
        vec![
            cx.subscribe_in(&self.password, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) && !this.service.is_unlocked() {
                    this.submit(window, cx);
                }
            }),
            cx.subscribe_in(&self.auto_lock, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.save_auto_lock(cx);
                }
            }),
        ]
    }
    pub(super) fn password(&self, cx: &App) -> Secret {
        Secret::new(self.password.read(cx).value().to_string())
    }
    pub(super) fn confirm_matches(&self, password: &Secret, cx: &App) -> bool {
        password.expose() == self.confirm.read(cx).value().as_ref()
    }
    pub(super) fn clear_passwords(&mut self, window: &mut Window, cx: &mut App) {
        self.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.confirm
            .update(cx, |input, cx| input.set_value("", window, cx));
    }
    pub(super) fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.password.read(cx).focus_handle(cx)
    }
    pub(super) fn auto_lock_minutes(&mut self, cx: &App) -> Option<u32> {
        let text = self.auto_lock.read(cx).value().trim().to_owned();
        match text.parse::<u32>() {
            Ok(minutes) if (1..=1440).contains(&minutes) => {
                self.auto_lock_error = None;
                Some(minutes)
            }
            _ => {
                self.auto_lock_error = Some("Enter a number of minutes from 1 to 1440.".into());
                None
            }
        }
    }
    pub(super) fn sync_auto_lock(&mut self, minutes: u32, window: &mut Window, cx: &mut App) {
        let minutes = minutes.to_string();
        let focused = self.auto_lock.read(cx).focus_handle(cx).is_focused(window);
        if !focused && self.auto_lock.read(cx).value() != minutes.as_str() {
            self.auto_lock
                .update(cx, |input, cx| input.set_value(minutes, window, cx));
        }
    }
    pub(super) fn password_input(&self) -> Input {
        Input::new(&self.password)
    }
    pub(super) fn confirm_input(&self) -> Input {
        Input::new(&self.confirm)
    }
    pub(super) fn auto_lock_input(&self) -> Input {
        Input::new(&self.auto_lock)
            .id("vault-auto-lock-input")
            .small()
    }
    pub(super) fn auto_lock_error(&self) -> Option<SharedString> {
        self.auto_lock_error.clone()
    }
}

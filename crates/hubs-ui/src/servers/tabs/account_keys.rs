//! The state of the account keys on this device, with the password prompt that opens them and
//! the button that closes them.
//!
//! The keys open the account's vault (saved servers and their credentials); chats and agents
//! never need them. They follow their own lock policy (`[hubs] key_lock_minutes`), not the
//! local vault's. This view never decides what the password opens: it hands the typed secret
//! to the service and reports the outcome.

use gpui_kit::{
    ClickEvent, Context, Entity, Focusable as _, Task, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_hubs::Hubs;
use nocterm_hubs_model::HubId;
use nocterm_session::Secret;

use crate::layout::{field, note, problem};
use crate::messages::account_key_problem;

/// The lock of the account keys of the server that is shown.
pub(in crate::servers) struct AccountKeys {
    hubs: Entity<Hubs>,
    hub: Option<HubId>,
    /// Created with the first frame, because a field needs a window.
    password: Option<Entity<InputState>>,
    /// Why the last attempt to unlock failed, as a sentence.
    refusal: Option<String>,
    unlocking: Option<Task<()>>,
}

impl AccountKeys {
    pub(in crate::servers) fn new(hubs: Entity<Hubs>) -> Self {
        Self {
            hubs,
            hub: None,
            password: None,
            refusal: None,
            unlocking: None,
        }
    }

    /// Shows the keys of `hub`; another server forgets the last refusal.
    pub(in crate::servers) fn show(&mut self, hub: &HubId, cx: &mut Context<Self>) {
        if self.hub.as_ref() != Some(hub) {
            self.hub = Some(hub.clone());
            self.refusal = None;
            self.unlocking = None;
            cx.notify();
        }
    }

    fn lock(&mut self, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        self.hubs.read(cx).lock_account_keys(&hub);
        cx.notify();
    }

    fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(hub), Some(input)) = (self.hub.clone(), self.password.clone()) else {
            return;
        };
        let typed = input.read(cx).value().to_string();
        input.update(cx, |input, cx| input.set_value("", window, cx));
        let unlocking = self
            .hubs
            .read(cx)
            .unlock_account_keys(&hub, Secret::new(typed));
        self.refusal = None;
        self.unlocking = Some(cx.spawn(async move |this, cx| {
            let result = unlocking.await;
            this.update(cx, |keys, cx| {
                keys.refusal = result.err().map(|error| account_key_problem(&error));
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        if self.password.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("Password")
            });
            // The prompt appears because the user needs it: type at once.
            window.focus(&input.focus_handle(cx), cx);
            self.password = Some(input);
        }
        let input = self.password.clone().expect("created above");
        let busy = self.unlocking.is_some();
        v_flex()
            .id("account-keys-locked")
            .test_support()
            .gap_2()
            .child(note(
                "The account keys are closed on this device. Type your password (or the vault passphrase of an account that signs in through a provider) to open them.",
                cx,
            ))
            .child(field("Password", Input::new(&input)))
            .children(
                self.refusal
                    .clone()
                    .map(|text| div().id("account-keys-refused").test_support().child(problem(text, cx))),
            )
            .child(
                h_flex().child(
                    Button::new("account-keys-unlock")
                        .primary()
                        .label("Unlock")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.unlock(window, cx);
                        })),
                ),
            )
            .into_any_element()
    }

    fn open(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        h_flex()
            .id("account-keys-open")
            .test_support()
            .gap_3()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Account keys are open on this device."),
            )
            .child(
                Button::new("account-keys-lock")
                    .small()
                    .label("Lock account keys")
                    .on_click(cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| {
                        this.lock(cx);
                    })),
            )
            .into_any_element()
    }
}

impl Render for AccountKeys {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.hub.clone() {
            None => div().into_any_element(),
            Some(hub) if self.hubs.read(cx).account_keys_open(&hub) => self.open(cx),
            Some(_) => self.prompt(window, cx),
        }
    }
}

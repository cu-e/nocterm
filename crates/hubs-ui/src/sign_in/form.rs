//! The text fields of the sign-in dialog.

use gpui_kit::{AppContext as _, Context, Entity, Window, component::input::InputState};
use nocterm_session::Secret;

/// The address, fingerprint, user name, password, vault passphrase and recovery key fields. A
/// secret is read out of its field once and the field is emptied, so a failed attempt starts
/// from a blank one.
pub(crate) struct SignInForm {
    address: Entity<InputState>,
    fingerprint: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    passphrase: Entity<InputState>,
    recovery_key: Entity<InputState>,
}

impl SignInForm {
    pub(crate) fn new<T: 'static>(window: &mut Window, cx: &mut Context<T>) -> Self {
        Self {
            address: cx.new(|cx| InputState::new(window, cx).placeholder("hub.example.com:7443")),
            fingerprint: cx
                .new(|cx| InputState::new(window, cx).placeholder("SHA-256 fingerprint, optional")),
            username: cx.new(|cx| InputState::new(window, cx).placeholder("User name")),
            password: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("Password")
            }),
            passphrase: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("Vault passphrase")
            }),
            recovery_key: cx.new(|cx| InputState::new(window, cx).placeholder("Recovery key")),
        }
    }

    pub(crate) fn address(&self) -> &Entity<InputState> {
        &self.address
    }

    pub(crate) fn fingerprint(&self) -> &Entity<InputState> {
        &self.fingerprint
    }

    pub(crate) fn username(&self) -> &Entity<InputState> {
        &self.username
    }

    pub(crate) fn password(&self) -> &Entity<InputState> {
        &self.password
    }

    pub(crate) fn passphrase(&self) -> &Entity<InputState> {
        &self.passphrase
    }

    pub(crate) fn recovery_key(&self) -> &Entity<InputState> {
        &self.recovery_key
    }

    pub(crate) fn address_text<T>(&self, cx: &Context<T>) -> String {
        self.address.read(cx).value().to_string()
    }

    pub(crate) fn fingerprint_text<T>(&self, cx: &Context<T>) -> String {
        self.fingerprint.read(cx).value().to_string()
    }

    pub(crate) fn username_text<T>(&self, cx: &Context<T>) -> String {
        self.username.read(cx).value().to_string()
    }

    /// The typed password; the field is emptied.
    pub(crate) fn take_password<T: 'static>(
        &self,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Secret {
        take(&self.password, window, cx)
    }

    /// The typed vault passphrase; the field is emptied.
    pub(crate) fn take_passphrase<T: 'static>(
        &self,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Secret {
        take(&self.passphrase, window, cx)
    }

    /// The typed recovery key; the field is emptied.
    pub(crate) fn take_recovery_key<T: 'static>(
        &self,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Secret {
        take(&self.recovery_key, window, cx)
    }
}

fn take<T: 'static>(
    field: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Secret {
    let secret = Secret::new(field.read(cx).value().to_string());
    field.update(cx, |input, cx| input.set_value("", window, cx));
    secret
}

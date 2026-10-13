//! Authentication selection and its native inputs.
use super::{ConnectionEditor, field, text, watch};
use gpui_kit::{App, Context, Entity, Subscription, Window, component::input::InputState};
use nocterm_connections_model::draft::{AuthKind, AuthenticationDraft};
use nocterm_connections_model::profiles::Profile;
use nocterm_session::Auth;
pub(super) struct AuthenticationFields {
    kind: AuthKind,
    key_path: Entity<InputState>,
    credential: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}
impl AuthenticationFields {
    pub(super) fn new(
        profile: Option<&Profile>,
        window: &mut Window,
        cx: &mut Context<ConnectionEditor>,
    ) -> Self {
        let (kind, path) = match profile.map(|p| &p.auth) {
            Some(Auth::Password) => (AuthKind::Password, String::new()),
            Some(Auth::Key { path }) => (AuthKind::Key, path.display().to_string()),
            _ => (AuthKind::Auto, String::new()),
        };
        let key_path = field("~/.ssh/id_ed25519", path, window, cx);
        let credential = field(
            "Credential ID copied from Vault (optional)",
            profile
                .and_then(|p| p.credential)
                .map(|id| id.to_string())
                .unwrap_or_default(),
            window,
            cx,
        );
        let subscriptions = [&key_path, &credential]
            .into_iter()
            .map(|input| watch(input, window, cx))
            .collect();
        Self {
            kind,
            key_path,
            credential,
            _subscriptions: subscriptions,
        }
    }
    pub(super) fn kind(&self) -> AuthKind {
        self.kind
    }
    pub(super) fn select(&mut self, kind: AuthKind) {
        self.kind = kind;
    }
    pub(super) fn key_path(&self) -> &Entity<InputState> {
        &self.key_path
    }
    pub(super) fn credential(&self) -> &Entity<InputState> {
        &self.credential
    }
    pub(super) fn snapshot(&self, cx: &App) -> AuthenticationDraft {
        AuthenticationDraft {
            kind: self.kind,
            key_path: text(&self.key_path, cx),
            credential: text(&self.credential, cx),
        }
    }
}

use super::render::field_row;
use gpui_kit::{
    AnyElement,
    component::{
        Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
    },
    prelude::*,
};
use nocterm_ui::form;

impl AuthenticationFields {
    fn render_auth_choice(&self, cx: &mut Context<ConnectionEditor>) -> AnyElement {
        let choice = |id: &'static str,
                      label: &'static str,
                      kind: AuthKind,
                      cx: &mut Context<ConnectionEditor>| {
            Button::new(id)
                .small()
                .ghost()
                .label(label)
                .selected(self.kind == kind)
                .on_click(cx.listener(move |this, _, window, cx| this.set_auth(kind, window, cx)))
        };
        h_flex()
            .gap_1()
            .child(choice("auth-auto", "Automatic", AuthKind::Auto, cx))
            .child(choice("auth-password", "Password", AuthKind::Password, cx))
            .child(choice("auth-key", "Key file", AuthKind::Key, cx))
            .into_any_element()
    }

    pub(super) fn page(&self, cx: &mut Context<ConnectionEditor>) -> Vec<AnyElement> {
        let hint = match self.kind {
            AuthKind::Auto => "Like ssh: the agent, then your default keys, then a password.",
            AuthKind::Password => "Always ask for the account's password.",
            AuthKind::Key => {
                "Sign in with one private key; its passphrase is asked for if it has one."
            }
        };
        let choice = self.render_auth_choice(cx);
        let mut sign_in = vec![form::stacked_row("Method", hint, choice, None, cx)];
        if self.kind == AuthKind::Key {
            sign_in.push(field_row(
                "Private key",
                "Path to the key file.",
                &self.key_path,
                cx,
            ));
        }
        vec![
            form::section("Sign in", sign_in, cx),
            form::section(
                "Saved password",
                [field_row(
                    "Credential ID",
                    "Saved from the sign-in prompt. Empty: ask each time.",
                    &self.credential,
                    cx,
                )],
                cx,
            ),
        ]
    }
}

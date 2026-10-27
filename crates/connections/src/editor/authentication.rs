//! Authentication selection and its native inputs.
use super::{ConnectionEditor, field, text, vault_key::VaultKeyField, watch};
use gpui_kit::{App, Context, Entity, Subscription, Window, component::input::InputState};
use nocterm_connections_model::draft::{AuthKind, AuthenticationDraft};
use nocterm_connections_model::profiles::Profile;
use nocterm_session::Auth;
use nocterm_ssh_keys::VaultSshKeys;
use std::sync::Arc;
pub(super) struct AuthenticationFields {
    kind: AuthKind,
    key_path: Entity<InputState>,
    credential: Entity<InputState>,
    vault_key: VaultKeyField,
    _subscriptions: Vec<Subscription>,
}
impl AuthenticationFields {
    pub(super) fn new(
        profile: Option<&Profile>,
        keys: Arc<dyn VaultSshKeys>,
        window: &mut Window,
        cx: &mut Context<ConnectionEditor>,
    ) -> Self {
        let (kind, path) = match profile.map(|p| &p.auth) {
            Some(Auth::Password) => (AuthKind::Password, String::new()),
            Some(Auth::Key { path }) => (AuthKind::Key, path.display().to_string()),
            Some(Auth::VaultKey { .. }) => (AuthKind::VaultKey, String::new()),
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
        let vault_key = VaultKeyField::new(keys, profile.map(|p| &p.auth), window, cx);
        Self {
            kind,
            key_path,
            credential,
            vault_key,
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
    pub(super) fn vault_key(&self) -> &VaultKeyField {
        &self.vault_key
    }
    pub(super) fn vault_key_mut(&mut self) -> &mut VaultKeyField {
        &mut self.vault_key
    }
    pub(super) fn credential(&self) -> &Entity<InputState> {
        &self.credential
    }
    pub(super) fn snapshot(&self, cx: &App) -> AuthenticationDraft {
        AuthenticationDraft {
            kind: self.kind,
            key_path: text(&self.key_path, cx),
            vault_key: self.vault_key.chosen(),
            credential: text(&self.credential, cx),
        }
    }
}

use super::render::field_row;
use gpui_kit::{
    AnyElement, ClipboardItem, SharedString,
    component::{
        Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_ui::{IconName, form};

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
            .child(choice(
                "auth-vault-key",
                "Vault key",
                AuthKind::VaultKey,
                cx,
            ))
            .into_any_element()
    }

    pub(super) fn page(&self, cx: &mut Context<ConnectionEditor>) -> Vec<AnyElement> {
        let hint = match self.kind {
            AuthKind::Auto => "Like ssh: the agent, then your default keys, then a password.",
            AuthKind::Password => "Always ask for the account's password.",
            AuthKind::Key => {
                "Sign in with one private key; its passphrase is asked for if it has one."
            }
            AuthKind::VaultKey => {
                "Sign in with a key kept in the vault. It follows the vault to your other devices."
            }
        };
        let choice = self.render_auth_choice(cx);
        let mut sign_in = vec![form::stacked_row("Method", hint, choice, None, cx)];
        match self.kind {
            AuthKind::Key => {
                sign_in.push(field_row(
                    "Private key",
                    "Path to the key file.",
                    &self.key_path,
                    cx,
                ));
                sign_in.push(form::row(
                    "Move to the vault",
                    "Stores this key in the vault and signs in with it from there.",
                    Button::new("vault-key-move")
                        .small()
                        .label("Move to vault")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.import_vault_key(window, cx)),
                        ),
                    cx,
                ));
            }
            AuthKind::VaultKey => sign_in.extend(self.vault_key_rows(cx)),
            AuthKind::Auto | AuthKind::Password => {}
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

impl AuthenticationFields {
    /// The vault's keys to choose from, and ways to add one.
    fn vault_key_rows(&self, cx: &mut Context<ConnectionEditor>) -> Vec<AnyElement> {
        let field = &self.vault_key;
        let keys = field.listed().iter().enumerate().map(|(index, key)| {
            Button::new(SharedString::from(format!("vault-key-{index}")))
                .small()
                .ghost()
                .label(format!("{} \u{b7} {}", key.label, key.fingerprint))
                .selected(field.is_chosen(key))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.authentication.vault_key_mut().choose_listed(index);
                    this.error = None;
                    cx.notify();
                }))
        });
        let list = v_flex().gap_1().children(keys).children(
            field
                .note()
                .map(|note| div().text_xs().child(note.to_owned())),
        );
        let mut rows = vec![form::stacked_row(
            "Key",
            "The keys the vault holds.",
            list.into_any_element(),
            None,
            cx,
        )];
        if let Some(fingerprint) = field.chosen_fingerprint() {
            rows.push(form::row(
                "Public key",
                fingerprint,
                Button::new("vault-key-copy")
                    .small()
                    .ghost()
                    .icon(IconName::Copy)
                    .label("Copy public key")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(chosen) = this.authentication.vault_key().chosen() {
                            cx.write_to_clipboard(ClipboardItem::new_string(chosen.public_key));
                        }
                    })),
                cx,
            ));
        }
        rows.push(field_row(
            "Key name",
            "How a new or imported key is named in the vault.",
            field.label(),
            cx,
        ));
        rows.push(form::row(
            "New key",
            "Creates an Ed25519 key in the vault.",
            Button::new("vault-key-create")
                .small()
                .label("Create")
                .on_click(cx.listener(|this, _, window, cx| this.create_vault_key(window, cx))),
            cx,
        ));
        rows.push(field_row(
            "Private key",
            "Path to a key file to import into the vault.",
            &self.key_path,
            cx,
        ));
        if field.passphrase_needed() {
            rows.push(field_row(
                "Passphrase",
                "The key file is protected; the vault keeps it without the passphrase.",
                field.passphrase(),
                cx,
            ));
        }
        rows.push(form::row(
            "Import",
            "Stores the key file in the vault.",
            Button::new("vault-key-import")
                .small()
                .label("Import")
                .on_click(cx.listener(|this, _, window, cx| this.import_vault_key(window, cx))),
            cx,
        ));
        rows
    }
}

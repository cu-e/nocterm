//! Authentication selection and its native inputs, and the password the connection signs in
//! with: typed here and kept in the window's vault, or one the vault already keeps.
use super::{
    ConnectionEditor, field, password::PasswordField, text, vault_key::VaultKeyField, watch,
};
use crate::{SavedPasswords, VaultPlace};
use gpui_kit::{App, Context, Entity, Subscription, Window, component::input::InputState};
use nocterm_connections_model::draft::{AuthKind, AuthenticationDraft};
use nocterm_connections_model::profiles::Profile;
use nocterm_session::{Auth, Secret};
use nocterm_ssh_keys::VaultSshKeys;
use std::sync::Arc;
pub(super) struct AuthenticationFields {
    kind: AuthKind,
    key_path: Entity<InputState>,
    password: Entity<PasswordField>,
    place: VaultPlace,
    /// The connection signs in with a key file, which a hub window offers only to keep it.
    key_file: bool,
    vault_key: Entity<VaultKeyField>,
    _subscriptions: Vec<Subscription>,
}
impl AuthenticationFields {
    pub(super) fn new(
        profile: Option<&Profile>,
        keys: Arc<dyn VaultSshKeys>,
        passwords: Arc<dyn SavedPasswords>,
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
        let place = passwords.place();
        let chosen = profile.and_then(|p| p.credential);
        let password = cx.new(|cx| {
            let mut field = PasswordField::new(passwords, chosen, window, cx);
            field.refresh(cx);
            field
        });
        let auth = profile.map(|p| &p.auth);
        let vault_key = cx.new(|cx| VaultKeyField::new(keys, auth, window, cx));
        let typed = password.read(cx).input().clone();
        let (label, passphrase) = {
            let field = vault_key.read(cx);
            (field.label().clone(), field.passphrase().clone())
        };
        let mut subscriptions: Vec<Subscription> = [&key_path, &typed, &label, &passphrase]
            .into_iter()
            .map(|input| watch(input, window, cx))
            .collect();
        subscriptions.push(cx.observe(&password, |_, _, cx| cx.notify()));
        subscriptions.push(cx.subscribe(&vault_key, |this, _, event, cx| {
            this.on_vault_key(event, cx)
        }));
        Self {
            kind,
            key_path,
            password,
            place,
            key_file: kind == AuthKind::Key,
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
    pub(super) fn vault_key(&self) -> &Entity<VaultKeyField> {
        &self.vault_key
    }
    pub(super) fn password(&self) -> &Entity<PasswordField> {
        &self.password
    }
    /// The field the page of the method starts with.
    pub(super) fn first_input(&self, cx: &App) -> Option<Entity<InputState>> {
        match self.kind {
            AuthKind::Auto | AuthKind::Password => Some(self.password.read(cx).input().clone()),
            AuthKind::Key => Some(self.key_path.clone()),
            AuthKind::VaultKey => None,
        }
    }
    /// The password typed to keep, for a method that signs in with one.
    pub(super) fn typed_password(&self, cx: &App) -> Option<Secret> {
        matches!(self.kind, AuthKind::Auto | AuthKind::Password)
            .then(|| self.password.read(cx).typed(cx))
            .flatten()
    }
    pub(super) fn snapshot(&self, cx: &App) -> AuthenticationDraft {
        AuthenticationDraft {
            kind: self.kind,
            key_path: text(&self.key_path, cx),
            vault_key: self.vault_key.read(cx).chosen(),
            credential: self.password.read(cx).chosen(),
        }
    }
}

use super::render::field_row;
use gpui_kit::component::input::Input;
use gpui_kit::{
    AnyElement, ClipboardItem, SharedString, TestSupportExt as _,
    component::{
        ActiveTheme as _, Icon, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
    rems,
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
        // A key file stays on this computer: a hub window keeps keys in the account's vault.
        let key_file = self.place == VaultPlace::ThisComputer || self.key_file;
        h_flex()
            .gap_1()
            .child(choice("auth-auto", "Automatic", AuthKind::Auto, cx))
            .child(choice("auth-password", "Password", AuthKind::Password, cx))
            .when(key_file, |row| {
                row.child(choice("auth-key", "Key file", AuthKind::Key, cx))
            })
            .child(choice(
                "auth-vault-key",
                "Vault key",
                AuthKind::VaultKey,
                cx,
            ))
            .into_any_element()
    }

    pub(super) fn page(&self, cx: &mut Context<ConnectionEditor>) -> Vec<AnyElement> {
        let account = self.place == VaultPlace::Account;
        let hint = match self.kind {
            AuthKind::Auto => {
                "Like ssh: this computer's agent and default keys, then the password."
            }
            AuthKind::Password => "Sign in with the account's password.",
            AuthKind::Key if account => {
                "A key file on this computer only. Move it to the vault to sign in from your other \
                 computers too."
            }
            AuthKind::Key => {
                "Sign in with one private key; its passphrase is asked for if it has one."
            }
            AuthKind::VaultKey if account => {
                "Sign in with a key kept in your account's vault on the hub. Every computer of \
                 the account signs in with it."
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
            AuthKind::Auto | AuthKind::Password => {
                sign_in.push(self.password_row(cx));
                sign_in.extend(self.saved_passwords_row(cx));
            }
        }
        vec![form::section("Sign in", sign_in, cx)]
    }

    /// The password to keep, and where it is kept.
    fn password_row(&self, cx: &mut Context<ConnectionEditor>) -> AnyElement {
        let account = self.place == VaultPlace::Account;
        let field = self.password.read(cx);
        let hint: SharedString = match (field.chosen_listed(), field.chosen().is_some(), account) {
            (Some(saved), _, _) if saved.servers > 1 => format!(
                "Signs in with \u{201c}{}\u{201d}, shared by {} servers. A new password changes \
                 it for all of them.",
                saved.label, saved.servers
            )
            .into(),
            (Some(saved), _, _) => format!(
                "Signs in with \u{201c}{}\u{201d} from the vault. Type a new one to replace it.",
                saved.label
            )
            .into(),
            (None, true, true) => {
                "Saved in your account's vault on the hub. Type a new one to replace it.".into()
            }
            (None, true, false) => {
                "Saved in this computer's vault. Type a new one to replace it.".into()
            }
            (None, false, true) => "Kept in your account's vault on the hub, encrypted end to \
                                    end: every computer of the account signs in with it. Empty: \
                                    asked when connecting."
                .into(),
            (None, false, false) => {
                "Kept in this computer's vault. Empty: asked when connecting.".into()
            }
        };
        let forget = field.chosen().is_some().then(|| {
            Button::new("editor-forget-password")
                .small()
                .ghost()
                .label("Forget")
                .tooltip("Ask for the password when connecting")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.authentication
                        .password()
                        .update(cx, |field, cx| field.forget(cx));
                }))
        });
        let input = field.input().clone();
        form::row(
            "Password",
            hint,
            h_flex()
                .gap_1()
                .child(
                    div().w(rems(13.)).child(
                        Input::new(&input)
                            .id("editor-field-Password")
                            .small()
                            .mask_toggle(),
                    ),
                )
                .children(forget),
            cx,
        )
    }

    /// The passwords the vault keeps, to sign in with one of them; none when it keeps none.
    fn saved_passwords_row(&self, cx: &mut Context<ConnectionEditor>) -> Option<AnyElement> {
        let field = self.password.read(cx);
        if field.listed().is_empty() {
            return None;
        }
        let chosen = field.chosen();
        let query = field.query().clone();
        let matching: Vec<(usize, String, usize, bool)> = field
            .matching(cx)
            .into_iter()
            .map(|(index, saved)| {
                let picked = chosen == Some(saved.id);
                (index, saved.label.clone(), saved.servers, picked)
            })
            .collect();
        let empty = matching.is_empty();
        let rows: Vec<AnyElement> = matching
            .into_iter()
            .map(|(index, label, servers, picked)| {
                saved_password_row(index, label, servers, picked, cx)
            })
            .collect();
        let list = v_flex()
            .w_full()
            .gap_0p5()
            .child(
                div().pb_1().child(
                    Input::new(&query)
                        .id("saved-passwords-search")
                        .small()
                        .prefix(Icon::new(IconName::Search).small())
                        .cleanable(true),
                ),
            )
            .children(rows)
            .when(empty, |list| {
                list.child(form::note("No saved password has this name.", cx))
            });
        Some(form::stacked_row(
            "Saved passwords",
            "Sign in with a password the vault keeps. One password can sign in to several \
             servers; add and rename them on the vault's Credentials tab.",
            list.into_any_element(),
            None,
            cx,
        ))
    }
}

/// One saved password to sign in with: its name and how many servers use it, left aligned,
/// the chosen one marked.
fn saved_password_row(
    index: usize,
    label: String,
    servers: usize,
    picked: bool,
    cx: &mut Context<ConnectionEditor>,
) -> AnyElement {
    let theme = cx.theme();
    let hover = theme.foreground.opacity(0.06);
    let servers = match servers {
        0 => "no server yet".to_owned(),
        1 => "1 server".to_owned(),
        n => format!("{n} servers"),
    };
    h_flex()
        .id(SharedString::from(format!("saved-password-{index}")))
        .test_support()
        .role(gpui_kit::Role::Button)
        .aria_label(label.clone())
        .w_full()
        .min_w_0()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .cursor_pointer()
        .text_sm()
        .when(picked, |row| row.bg(theme.secondary_active))
        .when(!picked, |row| row.hover(move |style| style.bg(hover)))
        .child(Icon::new(IconName::KeyRound).small().text_color(if picked {
            theme.primary
        } else {
            theme.muted_foreground
        }))
        .child(div().min_w_0().truncate().child(label))
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(servers),
        )
        .child(div().flex_1())
        .when(picked, |row| {
            row.child(Icon::new(IconName::Check).small().text_color(theme.primary))
        })
        .on_click(cx.listener(move |this, _, window, cx| {
            this.authentication
                .password()
                .update(cx, |field, cx| field.choose(index, window, cx));
        }))
        .into_any_element()
}

impl AuthenticationFields {
    /// The vault's keys to choose from, and ways to add one.
    fn vault_key_rows(&self, cx: &mut Context<ConnectionEditor>) -> Vec<AnyElement> {
        let field = self.vault_key.read(cx);
        let keys = field.listed().iter().enumerate().map(|(index, key)| {
            Button::new(SharedString::from(format!("vault-key-{index}")))
                .small()
                .ghost()
                .label(format!("{} \u{b7} {}", key.label, key.fingerprint))
                .selected(field.is_chosen(key))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.authentication
                        .vault_key()
                        .update(cx, |field, cx| field.choose_listed(index, cx));
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
                        if let Some(chosen) = this.authentication.vault_key().read(cx).chosen() {
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
        rows.push(self.import_row(cx));
        if field.passphrase_needed() {
            rows.push(field_row(
                "Passphrase",
                "The key file is protected; the vault keeps it without the passphrase.",
                field.passphrase(),
                cx,
            ));
        }
        rows
    }

    /// The key file to import, with its button beside the path so that the
    /// button stays where it was when the passphrase row appears below.
    fn import_row(&self, cx: &Context<ConnectionEditor>) -> AnyElement {
        form::row(
            "Private key",
            "Path to a key file to import into the vault.",
            h_flex()
                .gap_1()
                .child(
                    div().w(rems(13.)).child(
                        Input::new(&self.key_path)
                            .id("editor-field-Private key")
                            .small(),
                    ),
                )
                .child(
                    Button::new("vault-key-import")
                        .small()
                        .label("Import")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.import_vault_key(window, cx)),
                        ),
                ),
            cx,
        )
    }
}

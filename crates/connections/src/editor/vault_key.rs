//! The key kept in the vault that a connection signs in with: choosing one,
//! creating an Ed25519 key and importing a key file.
use super::{ConnectionEditor, field, watch};
use futures::future::BoxFuture;
use gpui_kit::{
    AppContext as _, Context, Entity, Subscription, Task, Window, component::input::InputState,
};
use nocterm_connections_model::draft::VaultKeyChoice;
use nocterm_session::{Auth, Secret};
use nocterm_ssh_keys::{
    KeyImportError, KeySource, PublicKeyText, StoredKey, VaultSshKeys, VaultSshKeysError,
};
use std::{path::PathBuf, sync::Arc};

/// Where the list of vault keys stands.
enum Listing {
    Loading,
    Ready,
    Unavailable(String),
}

pub(super) struct VaultKeyField {
    keys: Arc<dyn VaultSshKeys>,
    listed: Vec<StoredKey>,
    listing: Listing,
    chosen: Option<VaultKeyChoice>,
    /// The name a new or imported key is stored under.
    label: Entity<InputState>,
    /// Asked for only after the key turned out to be encrypted.
    passphrase: Entity<InputState>,
    passphrase_needed: bool,
    /// Bumped by every operation so that a superseded answer is dropped.
    epoch: u64,
    task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl VaultKeyField {
    pub(super) fn new(
        keys: Arc<dyn VaultSshKeys>,
        auth: Option<&Auth>,
        window: &mut Window,
        cx: &mut Context<ConnectionEditor>,
    ) -> Self {
        let chosen = match auth {
            Some(Auth::VaultKey {
                credential,
                public_key,
            }) => Some(VaultKeyChoice {
                credential: *credential,
                public_key: public_key.clone(),
            }),
            _ => None,
        };
        let label = field("Name of the new key", String::new(), window, cx);
        let passphrase = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Passphrase of the key file")
                .masked(true)
        });
        let subscriptions = vec![watch(&label, window, cx), watch(&passphrase, window, cx)];
        Self {
            keys,
            listed: Vec::new(),
            listing: Listing::Loading,
            chosen,
            label,
            passphrase,
            passphrase_needed: false,
            epoch: 0,
            task: None,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn chosen(&self) -> Option<VaultKeyChoice> {
        self.chosen.clone()
    }
    pub(super) fn label(&self) -> &Entity<InputState> {
        &self.label
    }
    pub(super) fn passphrase(&self) -> &Entity<InputState> {
        &self.passphrase
    }
    pub(super) fn passphrase_needed(&self) -> bool {
        self.passphrase_needed
    }
    pub(super) fn listed(&self) -> &[StoredKey] {
        &self.listed
    }
    pub(super) fn is_chosen(&self, key: &StoredKey) -> bool {
        self.chosen
            .as_ref()
            .is_some_and(|chosen| chosen.credential == key.credential)
    }
    /// What to tell the person about the list, when it is not simply shown.
    pub(super) fn note(&self) -> Option<&str> {
        match &self.listing {
            Listing::Loading => Some("Loading the vault's keys…"),
            Listing::Unavailable(message) => Some(message),
            Listing::Ready if self.listed.is_empty() => {
                Some("The vault holds no keys yet. Create one or import a key file.")
            }
            Listing::Ready => None,
        }
    }
    /// The fingerprint of the chosen key, from the list or its public half.
    pub(super) fn chosen_fingerprint(&self) -> Option<String> {
        let chosen = self.chosen.as_ref()?;
        match self
            .listed
            .iter()
            .find(|key| key.credential == chosen.credential)
        {
            Some(key) => Some(key.fingerprint.clone()),
            None => PublicKeyText::parse(&chosen.public_key)
                .ok()
                .map(|public| public.fingerprint().to_owned()),
        }
    }
    pub(super) fn choose_listed(&mut self, index: usize) {
        if let Some(key) = self.listed.get(index).cloned() {
            self.choose(&key);
        }
    }
    fn choose(&mut self, key: &StoredKey) {
        self.chosen = Some(VaultKeyChoice {
            credential: key.credential,
            public_key: key.public_key.as_str().to_owned(),
        });
    }

    /// Reads the vault's keys again.
    pub(super) fn refresh(&mut self, window: &mut Window, cx: &mut Context<ConnectionEditor>) {
        self.listing = Listing::Loading;
        let list = self.keys.list();
        let epoch = self.begin();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = list.await;
            let _ = this.update_in(cx, |this, _, cx| {
                let field = this.authentication.vault_key_mut();
                if field.epoch == epoch {
                    field.listing = match result {
                        Ok(keys) => {
                            field.listed = keys;
                            Listing::Ready
                        }
                        Err(error) => Listing::Unavailable(describe(&error)),
                    };
                    cx.notify();
                }
            });
        }));
    }

    /// Stores a new Ed25519 key and chooses it.
    pub(super) fn create(
        &mut self,
        fallback_label: &str,
        window: &mut Window,
        cx: &mut Context<ConnectionEditor>,
    ) {
        let label = self.label_or(fallback_label, cx);
        let stored = self.keys.generate(label);
        self.store(stored, window, cx);
    }

    /// Stores the key file at `path`, asking for its passphrase when it has one.
    pub(super) fn import(
        &mut self,
        fallback_label: &str,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<ConnectionEditor>,
    ) {
        let label = self.label_or(fallback_label, cx);
        let text = self.passphrase.read(cx).value().to_string();
        let passphrase = (!text.is_empty()).then(|| Secret::new(text));
        let stored = self.keys.import(label, KeySource::File(path), passphrase);
        self.store(stored, window, cx);
    }

    fn label_or(&self, fallback: &str, cx: &Context<ConnectionEditor>) -> String {
        let typed = self.label.read(cx).value().trim().to_owned();
        if typed.is_empty() {
            fallback.to_owned()
        } else {
            typed
        }
    }

    fn begin(&mut self) -> u64 {
        self.epoch = self.epoch.wrapping_add(1);
        self.epoch
    }

    fn store(
        &mut self,
        stored: BoxFuture<'static, Result<StoredKey, VaultSshKeysError>>,
        window: &mut Window,
        cx: &mut Context<ConnectionEditor>,
    ) {
        let epoch = self.begin();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = stored.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.authentication.vault_key_mut().epoch != epoch {
                    return;
                }
                match result {
                    Ok(key) => {
                        let field = this.authentication.vault_key_mut();
                        field.choose(&key);
                        field.listed.push(key);
                        field.listing = Listing::Ready;
                        field.passphrase_needed = false;
                        field
                            .passphrase
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.authentication
                            .select(nocterm_connections_model::draft::AuthKind::VaultKey);
                        this.error = None;
                    }
                    Err(error) => {
                        this.authentication.vault_key_mut().passphrase_needed |= matches!(
                            error,
                            VaultSshKeysError::Key(KeyImportError::PassphraseRequired)
                                | VaultSshKeysError::Key(KeyImportError::WrongPassphrase)
                        );
                        this.error = Some(describe(&error).into());
                    }
                }
                cx.notify();
            });
        }));
    }
}

/// What the person reads when the vault could not serve a key operation.
fn describe(error: &VaultSshKeysError) -> String {
    match error {
        VaultSshKeysError::Locked => "Unlock the vault to use its keys.".to_owned(),
        VaultSshKeysError::Offline => "The vault is offline.".to_owned(),
        VaultSshKeysError::Unavailable => "No vault is available.".to_owned(),
        VaultSshKeysError::Key(KeyImportError::PassphraseRequired) => {
            "The key file is protected: enter its passphrase and import again.".to_owned()
        }
        other => other.to_string(),
    }
}

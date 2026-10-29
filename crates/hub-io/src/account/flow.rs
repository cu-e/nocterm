//! Opening, creating and recovering the account keys around a sign-in.
//!
//! A first sign-in creates the keyset (one conditional write: a second device that races it
//! opens the keyset that won, it never overwrites it). A later sign-in opens it with the key
//! derived from the password. A password that does not open it (it was reset) leaves the keys
//! waiting for the recovery key. A keyset that does not authenticate although the key is right
//! was altered by the hub: that is an error and nothing is written.

use super::{
    key_admission::{KeyAdmission, KeySnapshot},
    source::{AccountSubject, AccountSubjects, CapturedMembership, LocalSourceProvider},
};
use crate::pool::captured::CapturedClient;
use std::sync::{Arc, Weak};
use tokio::runtime::Handle;

use futures::future::BoxFuture;
use nocterm_core::SharedCause;
use nocterm_hub_client::{Keyset, KeysetError, UnlockMethod as KeysetUnlock};
use nocterm_hub_proto::v1;
use nocterm_hubs_model::{HubError, HubId, KeyStatus};
use nocterm_keys::{LoginError, LoginSecrets, RecoveryKey, UnlockKdf, UnlockKey};
use nocterm_session::Secret;

use super::keyset_hub::KeysetHub;
use super::{Accounts, OpenKeys, Recovery};
use crate::cache::{AccountCache, KeysetCopy, UnlockMethod};
use crate::classify::remote;
use crate::directory::who_am_i;
use crate::pool::Pool;
use nocterm_hubs_model::{RecoveryKeyOffer, RecoveryKeyReissue};

/// The key work of a sign-in, for every hub.
pub(crate) struct KeyFlow {
    pool: Arc<Pool>,
    accounts: Arc<Accounts>,
    handle: Handle,
    weak: Weak<Self>,
}

/// How often the replacement of the recovery envelope is tried against a keyset that other
/// devices keep writing.
const RESEAL_ATTEMPTS: u32 = 4;

/// What a keyset write decided.
enum Creation {
    Created(nocterm_hub_client::NewKeyset),
    /// Another device created the keyset first.
    Raced,
}

/// A keyset that someone else already created is opened, not replaced.
fn creation(
    result: Result<nocterm_hub_client::NewKeyset, KeysetError>,
) -> Result<Creation, KeysetError> {
    match result {
        Ok(created) => Ok(Creation::Created(created)),
        Err(KeysetError::AlreadyExists) => Ok(Creation::Raced),
        Err(error) => Err(error),
    }
}

impl KeyFlow {
    pub(crate) fn new(pool: Arc<Pool>, accounts: Arc<Accounts>, handle: Handle) -> Arc<Self> {
        Arc::new_cyclic(|weak| Self {
            pool,
            accounts,
            handle,
            weak: weak.clone(),
        })
    }

    pub(crate) fn capture(&self, hub: &HubId) -> Result<Arc<KeyAdmission>, HubError> {
        let slot = self.accounts.slot(hub)?;
        let member = LocalSourceProvider::new(slot, self.pool.clone()).membership()?;
        KeyAdmission::native(
            member,
            Arc::new(CapturedClient::current(self.pool.clone(), hub.clone())?),
        )
    }

    pub(crate) fn published(
        &self,
        hub: &HubId,
        subject: &Arc<AccountSubject>,
    ) -> Result<Arc<KeyAdmission>, HubError> {
        let slot = self.accounts.slot(hub)?;
        let subjects: Arc<dyn AccountSubjects> = self.pool.clone();
        let member = CapturedMembership::published(&slot, &subjects, subject)?;
        KeyAdmission::native(
            member,
            Arc::new(CapturedClient::current(self.pool.clone(), hub.clone())?),
        )
    }

    pub(crate) fn bind_snapshot(
        &self,
        snapshot: KeySnapshot,
    ) -> Result<Arc<KeyAdmission>, HubError> {
        snapshot.check()?;
        let client = Arc::new(CapturedClient::current(
            self.pool.clone(),
            snapshot.member().hub().clone(),
        )?);
        KeyAdmission::bound(snapshot, client)
    }

    pub(crate) fn run<T, F>(
        &self,
        captured: Result<Arc<KeyAdmission>, HubError>,
        work: impl FnOnce(Arc<KeyAdmission>) -> F + Send + 'static,
    ) -> nocterm_hubs_model::HubFuture<T>
    where
        T: Send + 'static,
        F: std::future::Future<Output = Result<T, HubError>> + Send + 'static,
    {
        let owner = match captured {
            Ok(owner) => owner,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let returned = owner.clone();
        let task = crate::task::spawn(&self.handle, async move {
            owner.during(work(owner.clone())).await?
        });
        Box::pin(async move {
            let result = task.await.unwrap_or(Err(HubError::Stale));
            returned.check()?;
            result
        })
    }

    pub(crate) fn recovery_offer(&self, hub: &HubId) -> Result<RecoveryKeyReissue, HubError> {
        let owner = self.capture(hub)?;
        owner.keys()?;
        Ok(self.issuer(owner.snapshot()?))
    }

    pub(super) fn issuer(&self, snapshot: KeySnapshot) -> RecoveryKeyReissue {
        super::recovery::issuer(self.weak.clone(), snapshot)
    }

    pub(super) fn offer(
        &self,
        owner: &KeyAdmission,
        secret: Secret,
    ) -> Result<RecoveryKeyOffer, HubError> {
        Ok(super::recovery::offer(secret, owner.snapshot()?))
    }

    fn unconfirmed(&self, owner: &KeyAdmission) -> Result<bool, HubError> {
        match owner.cache() {
            Ok(cache) => Ok(cache.recovery_unconfirmed),
            Err(HubError::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn opened(&self, owner: &KeyAdmission) -> Result<KeyStatus, HubError> {
        if self.unconfirmed(owner)? {
            Ok(KeyStatus::RecoveryKeyUnconfirmed(
                self.issuer(owner.snapshot()?),
            ))
        } else {
            Ok(KeyStatus::Open)
        }
    }

    /// The keys after a password sign-in: the login password also derives the unlock key.
    pub(crate) async fn after_password(
        &self,
        owner: &KeyAdmission,
        client: &dyn KeysetHub,
        account_id: &str,
        username: &str,
        secrets: LoginSecrets,
    ) -> Result<KeyStatus, HubError> {
        owner.check()?;
        if owner.member().subject()?.account() != account_id {
            return Err(HubError::Stale);
        }
        let kdf = secrets.kdf().clone();
        let unlock = secrets.into_unlock();
        let method = UnlockMethod::Login {
            username: username.trim().to_owned(),
        };
        self.settle(owner, client, account_id, method, unlock, kdf)
            .await
    }

    /// Whether the keyset `record` is opened by something other than the login password. The
    /// record says so itself: the client never guesses it from how the user signed in.
    fn needs_vault_passphrase(
        &self,
        owner: &KeyAdmission,
        record: &v1::AccountKeyset,
    ) -> Result<bool, HubError> {
        match KeysetUnlock::of(record).map_err(|error| self.keyset_failure(owner, error))? {
            KeysetUnlock::LoginPassword => Ok(false),
            KeysetUnlock::VaultPassword(_) => Ok(true),
        }
    }

    /// The keys after a browser sign-in: nothing opens them until a passphrase is typed.
    pub(crate) fn after_browser(&self, owner: &KeyAdmission) -> Result<KeyStatus, HubError> {
        owner.check()?;
        match owner.keys() {
            Ok(_) => self.opened(owner),
            Err(HubError::KeysLocked) => Ok(KeyStatus::PassphraseNeeded),
            Err(error) => Err(error),
        }
    }

    /// Whether the recovery key made on this device was never confirmed as saved.
    pub(crate) fn recovery_key_unconfirmed(&self, hub: &HubId) -> bool {
        let Ok(slot) = self.accounts.slot(hub) else {
            return false;
        };
        slot.open()
            .is_some_and(|keys| slot.recovery_unconfirmed(keys.keyset().account_id()))
    }

    /// The count of changes to any hub's account keys (opened, dropped, replaced) so far.
    pub(crate) fn keys_revision(&self) -> u64 {
        self.accounts.notice().count()
    }

    /// Resolves with the count once it differs from `seen`.
    pub(crate) fn keys_changed(&self, seen: u64) -> BoxFuture<'static, u64> {
        let mut changes = self.accounts.notice().subscribe();
        Box::pin(async move {
            loop {
                let now = *changes.borrow_and_update();
                if now != seen || changes.changed().await.is_err() {
                    return now;
                }
            }
        })
    }

    pub(super) async fn reissue_recovery_key(
        &self,
        owner: &KeyAdmission,
    ) -> Result<Secret, HubError> {
        let keys = owner.keys()?;
        let client = owner.fresh().await?;
        self.reissue_with(owner, &client, keys.keyset()).await
    }

    async fn reissue_with(
        &self,
        owner: &KeyAdmission,
        client: &dyn KeysetHub,
        keyset: &Keyset,
    ) -> Result<Secret, HubError> {
        let hub = owner.member().hub();
        let before = owner.cache()?.recovery_unconfirmed;
        owner.keys()?;
        owner.begin_recovery_issue()?;
        let recovery = RecoveryKey::generate().map_err(remote)?;
        let written = self.reseal(owner, client, keyset, &recovery).await;
        match written {
            Ok(record) => {
                if let Err(error) = owner.update_cache(|cache| {
                    cache.keyset = KeysetCopy::of(&record);
                }) {
                    tracing::warn!(%error, hub = %hub, "the account cache cannot be written");
                }
                Ok(Secret::new(recovery.display().as_str()))
            }
            Err(error) => {
                if let Err(error) = owner.update_cache(|cache| cache.recovery_unconfirmed = before)
                {
                    tracing::warn!(%error, hub = %hub, "the account cache cannot be written");
                }
                Err(error)
            }
        }
    }

    /// Writes the recovery envelope conditional on the keyset version read just before. A
    /// device that wrote the keyset in between (another sign-in, a recovery) makes it conflict:
    /// the record is read again and the write repeated, a few times.
    async fn reseal(
        &self,
        owner: &KeyAdmission,
        client: &dyn KeysetHub,
        keyset: &Keyset,
        recovery: &RecoveryKey,
    ) -> Result<v1::AccountKeyset, HubError> {
        let mut attempt = 1;
        loop {
            match owner
                .during(client.reseal_recovery(keyset, recovery))
                .await?
            {
                Ok(record) => return Ok(record),
                Err(KeysetError::Hub(error))
                    if error.is_conflict() && attempt < RESEAL_ATTEMPTS =>
                {
                    attempt += 1;
                }
                Err(error) => return Err(self.keyset_failure(owner, error)),
            }
        }
    }

    /// Opens, or creates, the keys with the vault passphrase of an account without a password.
    pub(crate) async fn passphrase(
        &self,
        owner: &KeyAdmission,
        passphrase: Secret,
    ) -> Result<KeyStatus, HubError> {
        let client = owner.fresh().await?;
        let account = who_am_i(&client, owner).await?;
        self.open_with_passphrase(owner, &client, account.id.as_str(), passphrase)
            .await
    }

    async fn open_with_passphrase(
        &self,
        owner: &KeyAdmission,
        client: &dyn KeysetHub,
        account_id: &str,
        passphrase: Secret,
    ) -> Result<KeyStatus, HubError> {
        owner.check()?;
        if owner.member().subject()?.account() != account_id {
            return Err(HubError::Stale);
        }
        let record = match self.record(owner, client).await? {
            Some(record) => record,
            None => {
                let kdf = UnlockKdf::random().map_err(remote)?;
                let unlock = derive(&passphrase, &kdf).await?;
                let method = UnlockMethod::Passphrase;
                if let Some(created) = self
                    .try_create(owner, client, account_id, &method, &unlock, &kdf)
                    .await?
                {
                    return Ok(created);
                }
                // Another device made the keyset first, derived with a salt of its own: the
                // passphrase is tried again against the keyset that exists.
                self.record(owner, client)
                    .await?
                    .ok_or_else(|| damaged(KeysetError::Incomplete))?
            }
        };
        // The record says which password opens it; a keyset sealed with the login password
        // cannot be opened with a passphrase, whatever is typed.
        let kdf =
            match KeysetUnlock::of(&record).map_err(|error| self.keyset_failure(owner, error))? {
                KeysetUnlock::VaultPassword(kdf) => kdf,
                KeysetUnlock::LoginPassword => {
                    return Err(HubError::UnlockSourceMismatch {
                        needs_login_password: true,
                    });
                }
            };
        let unlock = derive(&passphrase, &kdf).await?;
        match Keyset::open(&record, account_id, &unlock) {
            Err(KeysetError::WrongUnlockKey) => {
                // Nothing says whether the passphrase was mistyped or is a new one: the
                // recovery key decides, and then the typed passphrase is kept.
                let fresh = UnlockKdf::random().map_err(remote)?;
                let unlock = derive(&passphrase, &fresh).await?;
                owner.recovering(Recovery {
                    account_id: account_id.to_owned(),
                    unlock,
                    kdf: fresh,
                    method: UnlockMethod::Passphrase,
                })?;
                Ok(KeyStatus::RecoveryNeeded)
            }
            other => {
                let keyset = other.map_err(|error| self.keyset_failure(owner, error))?;
                let unconfirmed = self.unconfirmed(owner)?;
                self.keep(owner, client, keyset, UnlockMethod::Passphrase, unconfirmed)
                    .await?;
                self.opened(owner)
            }
        }
    }

    /// Opens the keys with the recovery key, and makes the password typed at sign-in (or the
    /// passphrase) open them from now on.
    pub(crate) async fn recover(
        &self,
        owner: &KeyAdmission,
        recovery_key: Secret,
    ) -> Result<(), HubError> {
        let client = owner.fresh().await?;
        self.recover_with(owner, &client, recovery_key).await
    }

    async fn recover_with(
        &self,
        owner: &KeyAdmission,
        client: &dyn KeysetHub,
        recovery_key: Secret,
    ) -> Result<(), HubError> {
        let pending = owner.recovery()?;
        let key =
            RecoveryKey::parse(recovery_key.expose()).map_err(|_| HubError::RecoveryKeyInvalid)?;
        let keyset = owner
            .during(client.recover(&pending.account_id, &key, &pending.unlock, &pending.kdf))
            .await?
            .map_err(|error| match error {
                KeysetError::WrongRecoveryKey => HubError::RecoveryKeyInvalid,
                other => self.keyset_failure(owner, other),
            })?;
        // The person typed the recovery key, so it is the one they hold.
        self.keep(owner, client, keyset, pending.method.clone(), false)
            .await
    }

    async fn settle(
        &self,
        owner: &KeyAdmission,
        client: &dyn KeysetHub,
        account_id: &str,
        method: UnlockMethod,
        unlock: UnlockKey,
        kdf: UnlockKdf,
    ) -> Result<KeyStatus, HubError> {
        let record = match self.record(owner, client).await? {
            Some(record) => record,
            None => {
                let created = self
                    .try_create(owner, client, account_id, &method, &unlock, &kdf)
                    .await?;
                if let Some(created) = created {
                    return Ok(created);
                }
                // Another device made the keyset first: this one opens it, never replaces it.
                self.record(owner, client)
                    .await?
                    .ok_or_else(|| damaged(KeysetError::Incomplete))?
            }
        };
        if self.needs_vault_passphrase(owner, &record)? {
            // The login password cannot open it, and is not a wrong password either.
            return Ok(KeyStatus::PassphraseNeeded);
        }
        match Keyset::open(&record, account_id, &unlock) {
            Ok(keyset) => {
                let unconfirmed = self.unconfirmed(owner)?;
                self.keep(owner, client, keyset, method, unconfirmed)
                    .await?;
                self.opened(owner)
            }
            Err(KeysetError::WrongUnlockKey) => {
                owner.recovering(Recovery {
                    account_id: account_id.to_owned(),
                    unlock,
                    kdf,
                    method,
                })?;
                Ok(KeyStatus::RecoveryNeeded)
            }
            Err(error) => Err(self.keyset_failure(owner, error)),
        }
    }

    /// Creates the keyset and opens the vault with it. `None` when another device created the
    /// keyset first.
    async fn try_create(
        &self,
        owner: &KeyAdmission,
        client: &dyn KeysetHub,
        account_id: &str,
        method: &UnlockMethod,
        unlock: &UnlockKey,
        kdf: &UnlockKdf,
    ) -> Result<Option<KeyStatus>, HubError> {
        let made = creation(owner.during(client.create(account_id, unlock, kdf)).await?)
            .map_err(|error| self.keyset_failure(owner, error))?;
        match made {
            Creation::Created(new) => {
                // Marked before the key is shown: if it is never confirmed, the next time the
                // keys open another is offered.
                self.keep(owner, client, new.keyset, method.clone(), true)
                    .await?;
                Ok(Some(KeyStatus::Created(self.offer(
                    owner,
                    Secret::new(new.recovery.display().as_str()),
                )?)))
            }
            Creation::Raced => Ok(None),
        }
    }

    /// Opens the vault with `keyset` and writes the cache from the record the hub now holds.
    async fn keep(
        &self,
        owner: &KeyAdmission,
        client: &dyn KeysetHub,
        keyset: Keyset,
        method: UnlockMethod,
        unconfirmed: bool,
    ) -> Result<(), HubError> {
        let record = self
            .record(owner, client)
            .await?
            .ok_or_else(|| damaged(KeysetError::Incomplete))?;
        let mut cache = AccountCache::new(
            client.hub_uid(),
            keyset.account_id().to_owned(),
            method,
            &record,
        );
        cache.recovery_unconfirmed = unconfirmed;
        // What this device pinned for the account survives a new sign-in.
        match owner.cache() {
            Ok(earlier) => cache.inherit_pins(&earlier),
            Err(HubError::NotFound) => {}
            Err(error) => return Err(error),
        }
        let keys = OpenKeys::pinning(keyset, cache.pinned());
        owner.install(keys, cache)
    }

    async fn record(
        &self,
        owner: &KeyAdmission,
        client: &dyn KeysetHub,
    ) -> Result<Option<v1::AccountKeyset>, HubError> {
        owner
            .during(client.record())
            .await?
            .map_err(|error| owner.failure(error))
    }

    fn keyset_failure(&self, owner: &KeyAdmission, error: KeysetError) -> HubError {
        match error {
            KeysetError::Hub(error) => owner.failure(error),
            other => keyset_error(other),
        }
    }
}

/// A keyset failure that is not a hub call failing.
fn keyset_error(error: KeysetError) -> HubError {
    match error {
        KeysetError::Damaged(_)
        | KeysetError::Kdf(_)
        | KeysetError::Incomplete
        | KeysetError::PublicKey(_)
        | KeysetError::AlreadyExists
        | KeysetError::WrongRecoveryKey
        | KeysetError::WrongUnlockKey
        | KeysetError::UnsupportedUnlock { .. } => damaged(error),
        KeysetError::WrongUnlockSource { stored, .. } => HubError::UnlockSourceMismatch {
            needs_login_password: matches!(stored, nocterm_keys::UnlockSource::LoginPassword),
        },
        other => remote(other),
    }
}

fn damaged(error: KeysetError) -> HubError {
    HubError::KeysetDamaged {
        cause: SharedCause::new(error),
    }
}

/// The unlock key of a vault passphrase. Key derivation is slow and memory hard, so it does not
/// run on a thread that serves other hub calls.
async fn derive(passphrase: &Secret, kdf: &UnlockKdf) -> Result<UnlockKey, HubError> {
    let (passphrase, kdf) = (passphrase.clone(), kdf.clone());
    let derived = tokio::task::spawn_blocking(move || {
        UnlockKey::from_vault_password(passphrase.expose(), &kdf)
    })
    .await
    .map_err(remote)?;
    derived.map_err(|error| match error {
        LoginError::PasswordTooShort { .. } | LoginError::EmptyUsername => {
            HubError::InvalidVaultPassphrase
        }
        other => remote(other),
    })
}

#[cfg(test)]
mod tests;

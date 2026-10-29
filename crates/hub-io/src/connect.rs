//! Connecting to a hub and signing in or out.

use std::sync::Arc;

use futures::channel::oneshot;
use nocterm_hub_client::{HubClient, Session};
use nocterm_hubs_model::{
    Account, HubEndpointSpec, HubError, HubId, KeyStatus, SignedIn, TrustPolicy,
};
use nocterm_keys::DeviceKey;
use nocterm_session::Secret;

use crate::account::{KeyFlow, key_admission::KeyAdmission};
use crate::classify::{Method, classify_sign_in, explain_connect};
use crate::credentials::{CredentialFile, CredentialOrigin, LoginPublication};
use crate::directory::who_am_i;
use crate::endpoint::endpoint;
use crate::pool::Pool;
use crate::pool::observation::LoginObservation;
use zeroize::Zeroize as _;

mod login;
mod request;
mod sign_out;
mod start;
pub(crate) use request::Reconnect;
pub(crate) use sign_out::SignOut;
pub(crate) use start::LoginStart;

/// Connects to hubs and keeps the credentials of the ones the device signs in to.
pub(crate) struct Connector {
    pool: Arc<Pool>,
    keys: Arc<KeyFlow>,
}

/// A sign-in under way: the client and the file its tokens will be kept in. A file created for
/// a sign-in that did not finish is removed, so a hub never signed in to leaves no key behind.
struct Attempt {
    hub: HubId,
    file: CredentialFile,
    client: HubClient,
    trust: TrustPolicy,
    created: bool,
    finished: bool,
    origin: CredentialOrigin,
    observed: Arc<LoginObservation>,
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if self.created && !self.finished {
            let _ = self.file.erase_unpublished(self.origin.public_key());
        }
    }
}

impl Connector {
    pub(crate) fn new(pool: Arc<Pool>, keys: Arc<KeyFlow>) -> Self {
        Self { pool, keys }
    }

    pub(crate) fn reconnect(&self, spec: HubEndpointSpec) -> Result<Reconnect, HubError> {
        Reconnect::capture(self.pool.clone(), spec)
    }
    pub(crate) fn begin(&self, spec: HubEndpointSpec) -> Result<LoginStart, HubError> {
        LoginStart::capture(self.pool.clone(), spec)
    }
    pub(crate) fn sign_out(&self, hub: &HubId) -> Result<SignOut, HubError> {
        SignOut::capture(self.pool.clone(), hub.clone())
    }

    /// Keeps the tokens of a finished sign-in and reads who signed in, with the client that
    /// signed in.
    async fn complete(
        &self,
        mut attempt: Attempt,
        mut session: Session,
    ) -> Result<(Account, HubClient, Arc<KeyAdmission>), HubError> {
        let client = attempt.client.clone();
        let published = self.pool.publish_login(
            attempt.hub.clone(),
            attempt.trust,
            client.clone(),
            LoginPublication::new(&attempt.file, &session, &attempt.origin),
            &attempt.observed.snapshot(),
        );
        session.access_token.zeroize();
        session.refresh_token.zeroize();
        let published = published?;
        // After this point even a stale result never owns an unpublished file.
        attempt.finished = true;
        let (_, subject) = published.into_parts();
        attempt.observed.became(&subject)?;
        let owner = self.keys.published(&attempt.hub, &subject)?;
        let account = who_am_i(&client, &owner).await?;
        Ok((account, client, owner))
    }

    pub(crate) async fn password(
        &self,
        start: LoginStart,
        username: String,
        password: Secret,
    ) -> Result<SignedIn, HubError> {
        let attempt = start.open().await?;
        let hub = attempt.hub.clone();
        let signed_in = attempt
            .observed
            .during(
                attempt
                    .client
                    .password_login(&username, password.expose(), self.pool.device()),
            )
            .await?
            .map_err(|error| classify_sign_in(hub.address(), Method::Password, error))?;
        let secrets = signed_in.secrets;
        let (account, client, owner) = self.complete(attempt, signed_in.session).await?;
        // The login password also derives the key that opens the account's keyset.
        let keys =
            self.keys
                .after_password(&owner, &client, account.id.as_str(), &username, secrets);
        let keys = owner.during(keys).await??;
        Ok(SignedIn {
            account,
            keys,
            login: login::receipt(&self.pool, owner.member().clone()),
        })
    }

    /// Signs in through the browser. `browser` receives the address to open.
    pub(crate) async fn oidc(
        &self,
        start: LoginStart,
        provider: String,
        browser: oneshot::Sender<String>,
    ) -> Result<SignedIn, HubError> {
        let attempt = start.open().await?;
        let hub = attempt.hub.clone();
        let observed = attempt.observed.clone();
        let session = attempt
            .observed
            .during(
                attempt
                    .client
                    .oidc_login(&provider, self.pool.device(), |url| {
                        if observed.check().is_ok() {
                            let _ = browser.send(url.to_owned());
                        }
                    }),
            )
            .await?
            .map_err(|error| classify_sign_in(hub.address(), Method::Oidc, error))?;
        let (account, _, owner) = self.complete(attempt, session).await?;
        let keys: KeyStatus = self.keys.after_browser(&owner)?;
        Ok(SignedIn {
            account,
            keys,
            login: login::receipt(&self.pool, owner.member().clone()),
        })
    }

    /// Explicit removal of the configured hub is separate from an account worker.
    pub(crate) fn forget(&self, hub: &HubId) -> Result<(), HubError> {
        self.pool.forget_hub(hub)
    }
}

/// A client for the hub with the decided trust; a failure says why the hub was not reached.
async fn open(
    spec: &HubEndpointSpec,
    key: DeviceKey,
    session: Option<Session>,
) -> Result<HubClient, HubError> {
    let endpoint = endpoint(spec)?;
    match HubClient::connect(&endpoint, key, session).await {
        Ok(client) => Ok(client),
        Err(error) => Err(explain_connect(spec, error).await),
    }
}

#[cfg(test)]
mod tests;

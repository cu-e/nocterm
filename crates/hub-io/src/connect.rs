//! Connecting to a hub and signing in or out.

use std::sync::Arc;

use futures::channel::oneshot;
use nocterm_hub_client::{ClientError, HubClient, Session, StatusCode};
use nocterm_hub_proto::v1;
use nocterm_hubs_model::{Account, HubEndpointSpec, HubError, HubId, HubInfo, TrustPolicy};
use nocterm_keys::DeviceKey;
use nocterm_session::Secret;

use crate::classify::{
    Method, classify_sign_in, credential_file, credentials, explain_connect, remote,
};
use crate::convert;
use crate::credentials::CredentialFile;
use crate::directory::who_am_i;
use crate::endpoint::endpoint;
use crate::pool::Pool;

/// Connects to hubs and keeps the credentials of the ones the device signs in to.
pub(crate) struct Connector {
    pool: Arc<Pool>,
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
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if self.created && !self.finished {
            let _ = self.file.erase();
        }
    }
}

impl Connector {
    pub(crate) fn new(pool: Arc<Pool>) -> Self {
        Self { pool }
    }

    /// Connects with the decided trust, reads what the hub says of itself and renews the
    /// tokens of a device that is signed in. A hub keeps its client while it is trusted the
    /// same way, so renewing goes through one turn however often this is called.
    pub(crate) async fn connect(&self, spec: HubEndpointSpec) -> Result<HubInfo, HubError> {
        let hub = HubId::from(spec.address.clone());
        if let Some(client) = self.answering(&hub, &spec).await {
            return self.renewed(&hub, spec.trust, client).await;
        }
        let file = self.pool.file(&spec.address)?;
        let stored = file.load().map_err(credential_file)?;
        let (key, session) = match stored {
            Some(device) => (device.key, device.session),
            None => (DeviceKey::generate().map_err(remote)?, None),
        };
        let has_session = session.is_some();
        let client = open(&spec, key, session).await?;
        if !has_session {
            self.pool.evict(&hub);
            return Ok(convert::hub_info(client.info(), false));
        }
        client.persist_with(Arc::new(file));
        self.renewed(&hub, spec.trust, client).await
    }

    /// The pooled client of the hub, if the hub still answers as the one it connected to.
    async fn answering(&self, hub: &HubId, spec: &HubEndpointSpec) -> Option<HubClient> {
        let client = self.pool.connected(hub, spec.trust)?;
        let now = client.auth().get_hub_info(v1::Empty {}).await.ok()?;
        let (now, before) = (now.into_inner(), client.info());
        (now.hub_id == before.hub_id && now.protocol_version == before.protocol_version)
            .then_some(client)
    }

    /// Renews the tokens of `client`, which holds a session, and pools it while they work.
    async fn renewed(
        &self,
        hub: &HubId,
        trust: TrustPolicy,
        client: HubClient,
    ) -> Result<HubInfo, HubError> {
        let signed_in = match client.ensure_fresh().await {
            Ok(_) => true,
            Err(ClientError::Status {
                code: StatusCode::Unauthenticated,
                ..
            }) => {
                self.forget_tokens(hub)?;
                false
            }
            Err(error) => return Err(self.pool.failed(hub, error)),
        };
        let info = convert::hub_info(client.info(), signed_in);
        if signed_in {
            self.pool.insert(hub.clone(), trust, client);
        } else {
            self.pool.evict(hub);
        }
        Ok(info)
    }

    /// The hub no longer accepts the tokens: the device keeps its key and signs in again.
    fn forget_tokens(&self, hub: &HubId) -> Result<(), HubError> {
        self.pool
            .file(hub.address())?
            .clear_session()
            .map_err(credential_file)
    }

    async fn begin(&self, spec: &HubEndpointSpec) -> Result<Attempt, HubError> {
        let file = self.pool.file(&spec.address)?;
        let (device, created) = file.load_or_create().map_err(credential_file)?;
        let client = open(spec, device.key, device.session).await?;
        Ok(Attempt {
            hub: HubId::from(spec.address.clone()),
            file,
            client,
            trust: spec.trust,
            created,
            finished: false,
        })
    }

    /// Keeps the tokens of a finished sign-in and reads who signed in.
    async fn complete(&self, mut attempt: Attempt, session: Session) -> Result<Account, HubError> {
        attempt
            .file
            .save_session(&session)
            .map_err(credential_file)?;
        attempt.finished = true;
        let client = attempt.client.clone();
        client.persist_with(Arc::new(self.pool.file(attempt.hub.address())?));
        self.pool.insert(attempt.hub.clone(), attempt.trust, client);
        who_am_i(&attempt.client, &attempt.hub, &self.pool).await
    }

    pub(crate) async fn password(
        &self,
        spec: HubEndpointSpec,
        username: String,
        password: Secret,
    ) -> Result<Account, HubError> {
        let attempt = self.begin(&spec).await?;
        let signed_in = attempt
            .client
            .password_login(&username, password.expose(), self.pool.device())
            .await
            .map_err(|error| classify_sign_in(&spec.address, Method::Password, error))?;
        // The login also derives the key that opens the account's keyset. Nothing here needs it
        // yet, so it is dropped (and wiped) with `signed_in` once the tokens are kept.
        self.complete(attempt, signed_in.session).await
    }

    /// Signs in through the browser. `browser` receives the address to open.
    pub(crate) async fn oidc(
        &self,
        spec: HubEndpointSpec,
        provider: String,
        browser: oneshot::Sender<String>,
    ) -> Result<Account, HubError> {
        let attempt = self.begin(&spec).await?;
        let session = attempt
            .client
            .oidc_login(&provider, self.pool.device(), |url| {
                let _ = browser.send(url.to_owned());
            })
            .await
            .map_err(|error| classify_sign_in(&spec.address, Method::Oidc, error))?;
        self.complete(attempt, session).await
    }

    /// Ends this device's session on the hub if it can, and erases its credentials.
    pub(crate) async fn sign_out(&self, hub: &HubId) -> Result<(), HubError> {
        if let Some(client) = self.pool.evict(hub)
            && let Some(session) = client.session()
        {
            let request = v1::RevokeDeviceRequest {
                device_id: session.device_id,
            };
            if let Err(status) = client.directory().revoke_device(request).await {
                tracing::debug!(%status, hub = %hub, "the hub did not end the session");
            }
        }
        self.forget(hub)
    }

    /// Erases the credentials without contacting the hub.
    pub(crate) fn forget(&self, hub: &HubId) -> Result<(), HubError> {
        self.pool.evict(hub);
        self.pool.file(hub.address())?.erase().map_err(credentials)
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

//! Reconnect retains original device credentials before it is enqueued.
use super::open;
use crate::{
    classify::{credential_file, remote},
    convert,
    credentials::{CredentialFile, CredentialLoginId, Device},
    pool::{Pool, observation::LoginObservation},
    task::spawn,
};
use nocterm_hub_client::{ClientError, HubClient, StatusCode};
use nocterm_hub_proto::v1;
use nocterm_hubs_model::{HubEndpointSpec, HubError, HubFuture, HubId, HubInfo};
use nocterm_keys::DeviceKey;
use std::sync::Arc;
use tokio::runtime::Handle;

pub(crate) struct Reconnect {
    pool: Arc<Pool>,
    observed: Arc<LoginObservation>,
    spec: HubEndpointSpec,
    file: CredentialFile,
    device: Option<Device>,
    known: Option<(HubClient, Option<CredentialLoginId>)>,
    file_subject: Option<(String, String)>,
}
impl Reconnect {
    pub(crate) fn capture(pool: Arc<Pool>, spec: HubEndpointSpec) -> Result<Self, HubError> {
        let hub = HubId::from(spec.address.clone());
        let observed = LoginObservation::capture(pool.clone(), hub.clone())?;
        let known = pool.connected(&hub, spec.trust);
        let file = pool.file(&spec.address)?;
        let mut device = match file.load().map_err(credential_file)? {
            Some(device) => device,
            None => Device {
                key: DeviceKey::generate().map_err(remote)?,
                session: None,
                login: None,
            },
        };
        if let Some(subject) = observed.original_subject()? {
            let matches = device.session.as_ref().is_some_and(|session| {
                session.account_id == subject.account() && session.device_id == subject.device()
            });
            if !matches
                || known
                    .as_ref()
                    .is_some_and(|(_, login)| *login != device.login)
            {
                device.wipe_session();
                return Err(HubError::Stale);
            }
        }
        if let Err(error) = observed.check() {
            device.wipe_session();
            return Err(error);
        }
        let file_subject = device
            .session
            .as_ref()
            .map(|session| (session.account_id.clone(), session.device_id.clone()));
        Ok(Self {
            pool,
            observed,
            spec,
            file,
            device: Some(device),
            known,
            file_subject,
        })
    }
    pub(crate) fn start(self, handle: &Handle) -> HubFuture<HubInfo> {
        let observed = self.observed.clone();
        let returned = observed.clone();
        let task = spawn(handle, async move { observed.during(self.worker()).await? });
        Box::pin(async move {
            let result = task.await.unwrap_or(Err(HubError::Stale));
            returned.check()?;
            result
        })
    }
    async fn worker(mut self) -> Result<HubInfo, HubError> {
        if let Some((client, login)) = self.known.take() {
            let now = self
                .observed
                .during(client.auth().get_hub_info(v1::Empty {}))
                .await?;
            self.observed.check()?;
            if let Ok(now) = now {
                let now = now.into_inner();
                if now.hub_id == client.info().hub_id
                    && now.protocol_version == client.info().protocol_version
                {
                    return self.renewed(client, login).await;
                }
            }
        }
        let mut device = self.device.take().ok_or(HubError::Stale)?;
        let login = device.login;
        let store = device
            .session
            .as_ref()
            .map(|session| self.file.clone().bound(session, login));
        let client = self
            .observed
            .during(open(&self.spec, device.key, device.session.take()))
            .await??;
        self.observed.check()?;
        match store {
            Some(store) => client.persist_with(Arc::new(store)),
            None => return Ok(convert::hub_info(client.info(), false)),
        }
        self.renewed(client, login).await
    }
    async fn renewed(
        &self,
        client: HubClient,
        login: Option<CredentialLoginId>,
    ) -> Result<HubInfo, HubError> {
        match self.observed.during(client.ensure_fresh()).await? {
            Ok(_) => {
                self.observed.check()?;
                let info = convert::hub_info(client.info(), true);
                let next = self.pool.renew_observed(
                    self.observed.hub().clone(),
                    self.spec.trust,
                    client,
                    login,
                    &self.observed.snapshot(),
                )?;
                self.observed.published(next);
                Ok(info)
            }
            Err(error) => {
                let expired = matches!(
                    error,
                    ClientError::Status {
                        code: StatusCode::Unauthenticated,
                        ..
                    }
                );
                let revoked = matches!(error, ClientError::Revoked);
                if expired || revoked {
                    let subject = self
                        .file_subject
                        .as_ref()
                        .map(|(account, device)| (account.as_str(), device.as_str()));
                    self.observed.retire(&self.file, subject, login, revoked)?;
                    if expired {
                        return Ok(convert::hub_info(client.info(), false));
                    }
                }
                Err(crate::classify::classify(&self.spec.address, error))
            }
        }
    }
}
impl Drop for Reconnect {
    fn drop(&mut self) {
        if let Some(device) = &mut self.device {
            device.wipe_session();
        }
    }
}

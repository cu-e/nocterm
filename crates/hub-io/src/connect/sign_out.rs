//! Sign-out retains the original native member or original offline private-file origin.
use crate::{
    classify::credential_file,
    credentials::{CredentialFile, CredentialOrigin},
    pool::{Pool, observation::LoginObservation},
    task::spawn,
};
use nocterm_hub_client::HubClient;
use nocterm_hub_proto::v2;
use nocterm_hubs_model::{HubError, HubFuture, HubId};
use std::sync::Arc;
use tokio::runtime::Handle;

pub(crate) struct SignOut {
    observed: Arc<LoginObservation>,
    target: Target,
}
enum Target {
    Member {
        pool: Arc<Pool>,
        client: HubClient,
        subject: Arc<crate::account::source::AccountSubject>,
    },
    Offline {
        file: CredentialFile,
        origin: Option<CredentialOrigin>,
    },
}
impl SignOut {
    pub(crate) fn capture(pool: Arc<Pool>, hub: HubId) -> Result<Self, HubError> {
        let observed = LoginObservation::capture(pool.clone(), hub.clone())?;
        let target = if let Some(subject) = observed.original_subject()? {
            let client = pool.captured(&hub, &subject)?;
            Target::Member {
                pool,
                client,
                subject,
            }
        } else {
            let file = pool.file(hub.address())?;
            let origin = file.load().map_err(credential_file)?.map(|mut device| {
                let origin = device.origin();
                device.wipe_session();
                origin
            });
            Target::Offline { file, origin }
        };
        observed.check()?;
        Ok(Self { observed, target })
    }
    pub(crate) fn start(self, handle: &Handle) -> HubFuture<()> {
        let observed = self.observed.clone();
        let returned = observed.clone();
        let task = spawn(handle, async move { observed.during(self.worker()).await? });
        Box::pin(async move {
            let result = task.await.unwrap_or(Err(HubError::Stale));
            returned.check()?;
            result
        })
    }
    async fn worker(self) -> Result<(), HubError> {
        match self.target {
            Target::Member {
                pool,
                client,
                subject,
            } => {
                let (next, result) = pool
                    .end_membership(self.observed.hub(), &subject, false)?
                    .into_parts();
                self.observed.published(next);
                result?;
                // Signing out revokes this device on the hub.
                if let Err(error) = self
                    .observed
                    .during(client.account().logout(v2::Empty {}))
                    .await?
                {
                    tracing::debug!(%error,"the hub did not end the original device session");
                }
                Ok(())
            }
            Target::Offline { file, origin } => {
                let expected = self.observed.snapshot();
                let mut cleanup = None;
                self.observed.with_current(|| {
                    let retire = || {
                        cleanup = Some(
                            match self.observed.slot().end_observed(expected.receipt(), true) {
                                Ok(ended) => {
                                    let (receipt, result) = ended.into_parts();
                                    self.observed.ended_absence(receipt);
                                    result
                                }
                                Err(error) => Err(error),
                            },
                        );
                    };
                    let retired = match origin {
                        Some(ref origin) => file.retire_origin(origin, retire),
                        None => file.while_absent(retire),
                    }
                    .map_err(credential_file)?;
                    if !retired {
                        return Err(HubError::Stale);
                    }
                    cleanup.take().ok_or(HubError::Stale)??;
                    Ok(())
                })?;
                Ok(())
            }
        }
    }
}

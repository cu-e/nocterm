//! A native ready-generation capture, never public SDK authority or a new grant.
use super::{RunnerHubConnection, RunnerHubError};
use nocterm_hub_client::cloud::CloudRunner;
use std::{future::Future, sync::Arc};

pub(in crate::cloud) struct CapturedTransport {
    owner: Arc<RunnerHubConnection>,
    runner: CloudRunner,
    generation: Arc<()>,
}
impl CapturedTransport {
    pub(super) fn new(
        owner: Arc<RunnerHubConnection>,
        runner: CloudRunner,
        generation: Arc<()>,
    ) -> Self {
        Self {
            owner,
            runner,
            generation,
        }
    }
    pub(in crate::cloud) fn runner(&self) -> &CloudRunner {
        &self.runner
    }
    pub(in crate::cloud) fn check(&self) -> Result<(), RunnerHubError> {
        self.owner.check_identity()?;
        self.owner
            .session
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?
            .check_generation(&self.generation, super::runtime::now())?;
        self.owner.check_identity()
    }
    pub(in crate::cloud) async fn during<T>(
        &self,
        future: impl Future<Output = T>,
    ) -> Result<T, RunnerHubError> {
        let mut notice = self.owner.notice.subscribe();
        tokio::pin!(future);
        loop {
            self.check()?;
            let interval = self.owner.wait_interval()?;
            tokio::select! {
                biased;
                changed=notice.changed()=> { changed.map_err(|_| RunnerHubError::Unavailable)?; },
                result=&mut future=> { self.check()?; return Ok(result); },
                _=tokio::time::sleep(interval)=> {},
            }
        }
    }
}

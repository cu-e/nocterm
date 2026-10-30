//! One explicit native material ring and encrypted journal of the runner identity.
use crate::{RunnerError, config::Configuration};
use nocterm_cloud_runner::{DurableRunJournal, JournalLimits};
use nocterm_hub_io::cloud::{CloudRunnerIdentity, RunnerMaterialStore, RunnerMetadataCrypto};
use std::sync::Arc;

pub(super) struct DurableState {
    material: Arc<RunnerMaterialStore>,
    journal: DurableRunJournal,
}
impl DurableState {
    pub(super) fn open(
        config: &Configuration,
        identity: Arc<CloudRunnerIdentity>,
        create: bool,
    ) -> Result<Self, RunnerError> {
        let materials = config.state.join("material");
        let maximum =
            usize::try_from(config.limits.chats).map_err(|_| RunnerError::Value("limits.chats"))?;
        let material = if create {
            RunnerMaterialStore::create_new(
                &materials,
                config.binding.clone(),
                identity.clone(),
                maximum,
            )?
        } else {
            RunnerMaterialStore::open_existing(
                &materials,
                config.binding.clone(),
                identity.clone(),
                maximum,
            )?
        };
        let metadata = Arc::new(RunnerMetadataCrypto::new(identity.clone()));
        let limits = JournalLimits::new(
            config.limits.database_bytes,
            config.limits.chat_bytes,
            config.limits.records_per_chat,
            config.limits.chats,
        )?;
        let journal_path = config.state.join("journal");
        let journal = if create {
            nocterm_core::paths::ensure_private_dir(&journal_path).map_err(|error| {
                RunnerError::Configuration(nocterm_core::SharedCause::new(error))
            })?;
            DurableRunJournal::create_new(
                &journal_path,
                config.binding.clone(),
                material.journal_cipher(),
                metadata.clone(),
                metadata,
                limits,
            )?
        } else {
            DurableRunJournal::open_existing(
                &journal_path,
                config.binding.clone(),
                material.journal_cipher(),
                metadata.clone(),
                metadata,
                limits,
            )?
        };
        Ok(Self { material, journal })
    }
    pub(super) fn material(&self) -> Arc<RunnerMaterialStore> {
        self.material.clone()
    }
    pub(super) fn journal(&self) -> DurableRunJournal {
        self.journal.clone()
    }
    pub(super) fn cleanup_work_bytes(&self) -> usize {
        nocterm_cloud_model::journal::RunSessionFactCleanup::work_bytes(&self.journal.cleanup())
    }
}

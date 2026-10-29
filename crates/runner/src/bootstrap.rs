use crate::{
    RunnerError,
    config::{Configuration, ControlLimits},
};
use nocterm_cloud_model::{
    coordinator::IntakePoolLimits,
    journal::{PublicationBudget, PublicationWorkBudget},
};
use nocterm_cloud_runner::{DurableRunJournal, JournalLimits};
use nocterm_hub_io::cloud::{
    CloudRunnerIdentity, RunnerHubConnection, RunnerMaterialStore, RunnerMetadataCrypto,
};
use std::{path::PathBuf, sync::Arc};

/// Exact private native owners survive the whole process. No desktop login,
/// account private key or provider credential is an input to this bootstrap.
pub(crate) struct State {
    identity: Arc<CloudRunnerIdentity>,
    _material: Arc<RunnerMaterialStore>,
    journal: DurableRunJournal,
    connection: Arc<RunnerHubConnection>,
    account: [u8; 32],
    directory: PathBuf,
    limits: ControlLimits,
}
impl State {
    pub(crate) fn open(config: Configuration, create: bool) -> Result<Self, RunnerError> {
        nocterm_core::paths::ensure_private_dir(&config.state)
            .map_err(|error| RunnerError::Configuration(nocterm_core::SharedCause::new(error)))?;
        let identity_path = config.state.join("identity");
        let identity = Arc::new(if create {
            CloudRunnerIdentity::create_new(&identity_path, config.binding.clone())?
        } else {
            CloudRunnerIdentity::open_existing(&identity_path, config.binding.clone())?
        });
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
        // Session/rotation work is a finite separate native owner. No model or
        // transcript upload is admitted by this control-plane process.
        let work: Arc<dyn PublicationWorkBudget> = Arc::new(PublicationBudget::maintenance(
            IntakePoolLimits::new(64 * 1024, 2, 2)
                .map_err(|_| RunnerError::Value("session work budget"))?,
        ));
        let connection = RunnerHubConnection::new(
            config.trust,
            config.binding,
            identity.clone(),
            &config.state.join("session"),
            work,
        )?;
        Ok(Self {
            identity,
            _material: material,
            journal,
            connection,
            account: config.account,
            directory: config.state,
            limits: config.limits,
        })
    }
    pub(crate) fn connection(&self) -> &Arc<RunnerHubConnection> {
        &self.connection
    }
    pub(crate) fn identity(&self) -> Arc<CloudRunnerIdentity> {
        self.identity.clone()
    }
    pub(crate) fn heads(&self) -> nocterm_cloud_runner::DurableCurrentCloudHeads {
        self.journal.current_heads()
    }
    pub(crate) fn account(&self) -> [u8; 32] {
        self.account
    }
    pub(crate) fn directory(&self) -> &std::path::Path {
        &self.directory
    }
    pub(crate) fn limits(&self) -> &ControlLimits {
        &self.limits
    }
    pub(crate) fn print_identity(&self) -> Result<(), RunnerError> {
        let identity = self.identity.public_identity()?;
        println!(
            "mode=control-plane\nexecution_ready=false\nrunner={}\nencryption={}\nsigning={}\nproof={}",
            hex(identity.binding().runner().as_bytes()),
            hex(identity.encryption_key()),
            hex(identity.signing_key()),
            hex(identity.proof_key())
        );
        Ok(())
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runner_initialization_restores_same_private_identity_and_refuses_implicit_reset() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("runner");
        let initialized = State::open(crate::config::fixture(root.clone()), true).unwrap();
        let original = initialized.identity().public_identity().unwrap();
        assert!(State::open(crate::config::fixture(root.clone()), true).is_err());
        drop(initialized);
        let restored = State::open(crate::config::fixture(root.clone()), false).unwrap();
        let current = restored.identity().public_identity().unwrap();
        assert_eq!(current.encryption_key(), original.encryption_key());
        assert_eq!(current.signing_key(), original.signing_key());
        assert_eq!(current.proof_key(), original.proof_key());
        assert_eq!(current.binding(), original.binding());
        drop(restored);
        assert!(State::open(crate::config::fixture(root), true).is_err());
    }
}

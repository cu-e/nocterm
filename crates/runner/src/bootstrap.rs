mod durable;
use crate::{
    RunnerError,
    config::{Configuration, ControlLimits},
};
use nocterm_cloud_model::{
    coordinator::IntakePoolLimits,
    journal::{PublicationBudget, PublicationWorkBudget},
};
use nocterm_cloud_runner::DurableRunJournal;
use nocterm_hub_io::cloud::{CloudRunnerIdentity, RunnerHubConnection, RunnerMaterialStore};
use std::{path::PathBuf, sync::Arc};

/// Exact private native owners survive the whole process. No desktop login,
/// account private key or provider credential is an input to this bootstrap.
pub(crate) struct State {
    identity: Arc<CloudRunnerIdentity>,
    durable: durable::DurableState,
    connection: Arc<RunnerHubConnection>,
    account: [u8; 32],
    directory: PathBuf,
    limits: ControlLimits,
    execution: Option<crate::execution::ExecutionSetup>,
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
        let durable = durable::DurableState::open(&config, identity.clone(), create)?;
        let execution = crate::execution::ExecutionSetup::capture(
            config.execution,
            config.session_targets,
            durable.cleanup_work_bytes(),
        )?;
        // The execution sections can arrive after `init` (a managed host appends them at each
        // launch), and a daemon restore never creates the cursor itself, so a fresh
        // identity always gets its inbox now.
        if create {
            nocterm_hub_io::cloud::RunnerInbox::initialize_storage(
                &config.state.join("inbox"),
                &identity,
            )?;
        }
        // Session/rotation work is a finite separate native owner. No model or
        // transcript upload is admitted by this control-plane process.
        let work: Arc<dyn PublicationWorkBudget> = match &execution {
            Some(execution) => execution.transport_work(),
            None => Arc::new(PublicationBudget::maintenance(
                IntakePoolLimits::new(64 * 1024, 2, 2)
                    .map_err(|_| RunnerError::Value("session work budget"))?,
            )),
        };
        let connection = RunnerHubConnection::new(
            config.trust,
            config.binding,
            identity.clone(),
            &config.state.join("session"),
            work,
        )?;
        Ok(Self {
            identity,
            durable,
            connection,
            account: config.account,
            directory: config.state,
            limits: config.limits,
            execution,
        })
    }
    pub(crate) fn take_execution(&mut self) -> Option<crate::execution::ExecutionSetup> {
        self.execution.take()
    }
    pub(crate) fn material(&self) -> Arc<RunnerMaterialStore> {
        self.durable.material()
    }
    pub(crate) fn journal(&self) -> DurableRunJournal {
        self.durable.journal()
    }
    pub(crate) fn connection(&self) -> &Arc<RunnerHubConnection> {
        &self.connection
    }
    pub(crate) fn identity(&self) -> Arc<CloudRunnerIdentity> {
        self.identity.clone()
    }
    pub(crate) fn heads(&self) -> nocterm_cloud_runner::DurableCurrentCloudHeads {
        self.durable.journal().current_heads()
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
        crate::output::report(&format!(
            "mode=control-plane\nexecution_ready=false\nrunner={}\nencryption={}\nsigning={}\nproof={}\n",
            hex(identity.binding().runner().as_bytes()),
            hex(identity.encryption_key()),
            hex(identity.signing_key()),
            hex(identity.proof_key())
        ))
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
    #[test]
    fn an_initialized_runner_has_its_inbox_before_any_execution_is_configured() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("runner");
        drop(State::open(crate::config::fixture(root.clone()), true).unwrap());
        assert!(
            std::fs::read_dir(root.join("inbox"))
                .unwrap()
                .next()
                .is_some(),
            "the inbox cursor was not created by init"
        );
    }
}

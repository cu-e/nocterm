//! Actual daemon execution composition. Configuration never carries API secrets.
mod budgets;
#[cfg(target_os = "linux")]
mod compose;
pub(crate) mod config;
#[cfg(target_os = "linux")]
mod publisher;
#[cfg(target_os = "linux")]
mod runtime;
#[cfg(not(target_os = "linux"))]
#[path = "execution/unsupported.rs"]
mod runtime;
use crate::{RunnerError, sessions::config::SessionTargetConfiguration};
use budgets::ExecutionBudgets;
use config::ExecutionConfig;
use nocterm_cloud_model::journal::PublicationWorkBudget;
pub(crate) use runtime::ExecutionRuntime;
use std::sync::Arc;

/// Complete explicit configuration and its one shared bounded pool generation.
/// The daemon consumes this once; health/setup commands never start execution.
pub(crate) struct ExecutionSetup {
    configuration: ExecutionConfig,
    targets: SessionTargetConfiguration,
    budgets: ExecutionBudgets,
    /// `[ssh] egress` of an own runner; a managed boot's egress replaces it.
    ssh_egress: Vec<String>,
}
impl ExecutionSetup {
    pub(crate) fn capture(
        configuration: Option<ExecutionConfig>,
        targets: Option<SessionTargetConfiguration>,
        ssh_egress: Vec<String>,
        cleanup_bytes: usize,
    ) -> Result<Option<Self>, RunnerError> {
        match (configuration, targets) {
            (None, None) => Ok(None),
            (Some(configuration), Some(targets)) => {
                configuration.maximum_grants()?;
                targets.default_destination()?;
                Ok(Some(Self {
                    configuration,
                    targets,
                    budgets: ExecutionBudgets::new(cleanup_bytes)?,
                    ssh_egress,
                }))
            }
            _ => Err(RunnerError::Value("execution/session_targets")),
        }
    }
    pub(crate) fn transport_work(&self) -> Arc<dyn PublicationWorkBudget> {
        self.budgets.transport()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn execution() -> ExecutionConfig {
        toml::from_str(
            "maximum_provider_grants = 4\n[containment]\nworkspace = \"/private/account-workspace\"\nmaximum_address_space_bytes = 268435456\nmaximum_processes = 16\nmaximum_descriptors = 64\nmaximum_cpu_seconds = 60\nmaximum_file_bytes = 1048576\n[[providers]]\nid = \"provider\"\nprotocol = \"openai_chat\"\nendpoint = \"https://api.example.org/v1\"\n[[providers.models]]\nid = \"model\"\npricing_revision = \"revision1\"\nmaximum_request_bytes = 65536\nmaximum_output_tokens = 2048\nmaximum_call_cost_micros = 50000\n",
        )
        .unwrap()
    }
    fn targets(default: &str) -> SessionTargetConfiguration {
        let fp = |byte: &str| byte.repeat(32);
        toml::from_str(&format!(
            "default = \"{default}\"\n[[targets]]\nname = \"local\"\nhost_fingerprint = \"{}\"\nroute_fingerprint = \"{}\"\nprincipal = \"agent\"\nworkspace_fingerprint = \"{}\"\nauthentication_key = \"{}\"\ncredential_revision = \"{}\"\n[targets.connector]\nkind = \"local\"\nterminal = \"xterm-256color\"\ncolumns = 80\nrows = 24\n[targets.connector.launch]\nintegration = true\n",
            fp("01"),
            fp("02"),
            fp("03"),
            fp("04"),
            fp("05"),
        ))
        .unwrap()
    }
    #[test]
    fn a_daemon_without_execution_configuration_stays_a_control_plane() {
        assert!(
            ExecutionSetup::capture(None, None, Vec::new(), 1024)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn execution_without_session_targets_or_the_reverse_is_refused() {
        assert!(matches!(
            ExecutionSetup::capture(Some(execution()), None, Vec::new(), 1024),
            Err(RunnerError::Value("execution/session_targets"))
        ));
        assert!(matches!(
            ExecutionSetup::capture(None, Some(targets("local")), Vec::new(), 1024),
            Err(RunnerError::Value("execution/session_targets"))
        ));
    }
    #[test]
    fn complete_configuration_captures_one_bounded_setup_and_an_unknown_default_is_refused() {
        let setup =
            ExecutionSetup::capture(Some(execution()), Some(targets("local")), Vec::new(), 1024)
                .unwrap()
                .unwrap();
        assert!(Arc::ptr_eq(
            &setup.transport_work(),
            &setup.transport_work()
        ));
        assert!(matches!(
            ExecutionSetup::capture(
                Some(execution()),
                Some(targets("missing")),
                Vec::new(),
                1024
            ),
            Err(RunnerError::Value("session_targets.default"))
        ));
    }
}

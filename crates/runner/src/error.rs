use nocterm_core::SharedCause;

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error(
        "usage: nocterm-runner <init|enrol|run|health|refresh|show-public|managed> --config <private.toml> [--control stdin]; enrol reads its ticket and managed its boot document from stdin"
    )]
    Arguments,
    #[error("the private runner configuration could not be read")]
    Configuration(#[source] SharedCause),
    #[error("invalid runner configuration field: {0}")]
    Value(&'static str),
    #[error("the bounded enrollment ticket could not be read")]
    Ticket(#[source] SharedCause),
    #[error("the enrollment ticket must contain 1..256 ASCII non-whitespace bytes")]
    TicketFormat,
    #[error("the native control-plane runtime could not start")]
    Runtime(#[source] std::io::Error),
    #[error("native runner identity access failed")]
    Identity(#[from] nocterm_hub_io::cloud::CloudRunnerIdentityError),
    #[error("native runner material access failed")]
    Material(#[from] nocterm_hub_io::cloud::MaterialStoreError),
    #[error("the native runner hub operation failed")]
    Hub(#[from] nocterm_hub_io::cloud::RunnerHubError),
    #[error("native journal access failed")]
    Journal(#[from] nocterm_cloud_model::journal::JournalError),
    #[error("the runner chat directory could not be composed")]
    Directory(#[from] nocterm_hub_io::cloud::RunnerDirectoryError),
    #[error("the bounded native head responder failed")]
    Head(#[from] nocterm_hub_io::cloud::RunnerHeadError),
    #[error("the configured Cloud execution runtime failed")]
    Execution(#[from] nocterm_cloud_model::coordinator::CoordinatorError),
    #[error("the agent transfer runtime could not start")]
    Transfers(#[from] nocterm_transfer_runtime::TransferRuntimeError),
    #[cfg(target_os = "linux")]
    #[error("required native tool containment is unavailable")]
    Containment(#[from] nocterm_local::ContainmentError),
    #[error("the configured API model provider is unavailable")]
    Inference(#[from] nocterm_cloud_model::engine::EngineError),
    #[error("the runner clock is unavailable or moved backwards")]
    Clock,
    #[error("the pinned runner refresh credential has expired")]
    Expired,
    #[error("the configured control-plane operation timed out")]
    Timeout,
    #[error("the runner did not finish its actual native work before the shutdown deadline")]
    DrainTimeout,
    #[error("the private local control endpoint is unavailable")]
    Control(#[source] std::io::Error),
    #[error("the control-plane task stopped unexpectedly")]
    Task(#[source] tokio::task::JoinError),
    #[error("the managed boot document could not be read")]
    Boot(#[source] SharedCause),
    #[error("the managed boot document's SSH section is invalid")]
    BootSsh(#[from] nocterm_hub_io::runner_control::boot_ssh::BootSshError),
    #[error("the managed control channel failed and its tokens were revoked")]
    ControlChannel(#[source] nocterm_hub_io::runner_control::ChatAccessFrameError),
    #[error("the runner could not write its report")]
    Output(#[source] std::io::Error),
    #[error("this control-plane executable currently requires Unix")]
    Platform,
}

impl RunnerError {
    /// The error and every cause beneath it as one line, outermost first.
    ///
    /// Every frame is a typed variant with a static message or an operating
    /// system error; none formats a configuration value, token or path, so the
    /// whole chain is safe for the process's standard error.
    pub fn chain(&self) -> String {
        let mut line = self.to_string();
        let mut cause = std::error::Error::source(self);
        while let Some(error) = cause {
            line.push_str(": ");
            line.push_str(&error.to_string());
            cause = error.source();
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_cloud_model::coordinator::CoordinatorError;

    #[test]
    fn the_report_names_the_cause_beneath_the_execution_failure() {
        let error = RunnerError::Execution(CoordinatorError::Journal(
            nocterm_cloud_model::journal::JournalError::Corrupt,
        ));
        let line = error.chain();
        assert!(line.starts_with("the configured Cloud execution runtime failed: "));
        assert!(line.contains("the durable journal refused the operation: "));
        assert!(line.split(": ").count() >= 3);
    }

    #[test]
    fn an_error_without_a_cause_reports_only_itself() {
        assert_eq!(RunnerError::Clock.chain(), RunnerError::Clock.to_string());
    }
}

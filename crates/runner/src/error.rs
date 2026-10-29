use nocterm_core::SharedCause;

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error(
        "usage: nocterm-runner <init|enrol|run|health|refresh> --config <private.toml>; enrol reads its ticket from stdin"
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
    #[error("the bounded native head responder failed")]
    Head(#[from] nocterm_hub_io::cloud::RunnerHeadError),
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
    #[error("this control-plane executable currently requires Unix")]
    Platform,
}

//! What a managed start hands the daemon: either the legacy lease document or a
//! live control channel with the providers that use its tokens (ADR 0123).
use super::{Boot, BootV2, BrokeredProviders, ControlEnd, read_boot_v2, spawn_reader};
use crate::{RunnerError, config::Configuration, execution::config::ExecutionConfig};
use nocterm_cloud_model::identity::AccountScope;
use std::{fs::File, os::fd::AsFd, sync::Arc};
use tokio::sync::oneshot;

/// The document read from standard input, before any runtime exists.
pub(crate) enum Launch {
    /// Boot v1, read to EOF: control plane only.
    Legacy(Boot),
    /// Boot v2 frame read; the rest of standard input is the control channel.
    Controlled(BootV2, File),
}
impl Launch {
    pub(crate) fn read(control: bool, config: &Configuration) -> Result<Self, RunnerError> {
        if control {
            // The control channel exists to feed tool execution: it needs the
            // containment and session target, and no provider of its own.
            let brokered = config
                .execution
                .as_ref()
                .is_some_and(ExecutionConfig::is_brokered);
            // A managed runner's SSH egress is the host's, stated in boot v3.
            if !brokered || config.session_targets.is_none() || !config.ssh_egress.is_empty() {
                return Err(RunnerError::Value("managed execution configuration"));
            }
            // An unbuffered duplicate of fd 0: `Stdin` keeps its own buffer that is
            // never wiped, so token bytes must not pass through it (ADR 0123).
            let mut input = std::io::stdin()
                .as_fd()
                .try_clone_to_owned()
                .map(File::from)
                .map_err(|error| RunnerError::Boot(nocterm_core::SharedCause::new(error)))?;
            let boot = read_boot_v2(&mut input)?;
            return Ok(Self::Controlled(boot, input));
        }
        // Without a control channel the configuration is the pool host's alone.
        if config.execution.is_some() || config.session_targets.is_some() {
            return Err(RunnerError::Value("managed configuration"));
        }
        Ok(Self::Legacy(Boot::read(std::io::stdin().lock())?))
    }
}

/// A running control channel. Created inside the runtime, which hosts the leases.
pub(crate) struct Session {
    boot: BootV2,
    providers: Arc<BrokeredProviders>,
    closed: Option<oneshot::Receiver<ControlEnd>>,
}
impl Session {
    /// Starts the reader thread. From here every token the host sends reaches
    /// `providers`, and the end of the channel revokes them all.
    pub(crate) fn start(
        boot: BootV2,
        input: File,
        scope: &AccountScope,
    ) -> Result<Self, RunnerError> {
        let providers = Arc::new(BrokeredProviders::new(
            &boot,
            scope,
            tokio::runtime::Handle::current(),
        )?);
        let closed = spawn_reader(input, providers.tokens().clone())?;
        Ok(Self {
            boot,
            providers,
            closed: Some(closed),
        })
    }
    pub(crate) fn announce(&self, execution_ready: bool) -> Result<(), RunnerError> {
        crate::output::report(&self.boot.summary(execution_ready))
    }
    pub(crate) fn providers(&self) -> &Arc<BrokeredProviders> {
        &self.providers
    }
    /// The boot's SSH targets and egress for chat SSH bindings (ADR 0137).
    pub(crate) fn ssh(&self) -> crate::sessions::reach::BootSsh {
        crate::sessions::reach::BootSsh::new(self.boot.ssh_targets(), self.boot.ssh_egress())
    }
    /// Completes when the host closes the channel, which ends the lease. A
    /// channel that failed instead is reported once and never completes: the
    /// runner keeps serving, with every token revoked.
    pub(crate) async fn ended(&mut self) -> Result<(), RunnerError> {
        match self.closed.take() {
            Some(closed) => outcome(closed).await,
            None => std::future::pending().await,
        }
    }
}
async fn outcome(closed: oneshot::Receiver<ControlEnd>) -> Result<(), RunnerError> {
    match closed.await {
        Ok(ControlEnd::Eof) | Err(_) => Ok(()),
        Ok(ControlEnd::Refused(error)) => {
            use std::io::Write as _;
            // A closed stderr leaves nowhere else to report.
            let _ = writeln!(std::io::stderr(), "{}", error.chain());
            std::future::pending().await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn the_host_closing_the_channel_ends_the_lease_and_stops_the_runner() {
        let (ended, closed) = oneshot::channel();
        ended.send(ControlEnd::Eof).unwrap();
        assert!(outcome(closed).await.is_ok());
    }

    #[tokio::test]
    async fn a_channel_that_failed_keeps_the_runner_serving() {
        let (ended, closed) = oneshot::channel();
        let failure = nocterm_hub_io::runner_control::ChatAccessFrameError::Truncated;
        ended
            .send(ControlEnd::Refused(RunnerError::ControlChannel(failure)))
            .unwrap();
        let waited = tokio::time::timeout(Duration::from_millis(100), outcome(closed)).await;
        assert!(
            waited.is_err(),
            "a refused channel does not stop the runner"
        );
    }
}

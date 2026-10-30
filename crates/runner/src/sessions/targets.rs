//! Trusted native targets; signed fingerprints select, never construct, I/O.
use nocterm_cloud_model::{
    authority::{AuthenticationBinding, TargetBinding},
    coordinator::{CoordinatorError, SessionRequest, SessionStartup},
    journal::JournalCatalogBinding,
    login::SessionLoginAuthorizer,
};
use nocterm_core::{SharedCause, allocation::AllocationLease};
use nocterm_local::ContainedLocal;
use nocterm_session::{
    Auth, ConnectRequest, ConnectionOpenOptions, ConnectionSource, PtySize, Session, ShellLaunch,
};
use nocterm_ssh::{SshTransport, proof::SessionLoginAttempt};
use nocterm_ssh_keys::PublicKeyText;
use std::sync::Arc;

pub(crate) const TARGET_CAPTURE_BYTES: usize = 64 * 1024;

pub(crate) struct ConfiguredSessionTargets {
    binding: JournalCatalogBinding,
    entries: Vec<ConfiguredSessionTarget>,
}
pub(crate) struct ConfiguredSessionTarget {
    target: TargetBinding,
    authentication: AuthenticationBinding,
    connector: Connector,
}
enum Connector {
    Local {
        local: ContainedLocal,
        launch: ShellLaunch,
        term: String,
        size: PtySize,
    },
    Ssh {
        transport: Arc<SshTransport>,
        request: Box<ConnectRequest>,
        host: PublicKeyText,
    },
}
impl ConfiguredSessionTarget {
    pub(crate) fn local(
        target: TargetBinding,
        authentication: AuthenticationBinding,
        local: ContainedLocal,
        launch: ShellLaunch,
        term: String,
        size: PtySize,
    ) -> Result<Self, CoordinatorError> {
        validate_launch(&launch, &term, size)?;
        if !launch.integration {
            return Err(CoordinatorError::Unsupported);
        }
        Ok(Self {
            target,
            authentication,
            connector: Connector::Local {
                local,
                launch,
                term,
                size,
            },
        })
    }
    pub(crate) fn ssh(
        target: TargetBinding,
        authentication: AuthenticationBinding,
        transport: Arc<SshTransport>,
        request: ConnectRequest,
        host: PublicKeyText,
    ) -> Result<Self, CoordinatorError> {
        validate_launch(&request.launch, &request.term, request.size)?;
        if request.proxy != nocterm_session::ProxyConfig::Direct
            || !request.jumps.is_empty()
            || request.auth != Auth::Auto
            || request.target.user != target.principal().as_str()
            || request.target.host.is_empty()
            || request.target.host.len() > 253
            || request.target.port == 0
            || request.connect_timeout.is_zero()
            || request.connect_timeout > std::time::Duration::from_secs(60)
            || host.as_str().len() > 2048
            || host.fingerprint_bytes().map_err(backend)? != *target.host().as_bytes()
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Self {
            target,
            authentication,
            connector: Connector::Ssh {
                transport,
                request: Box::new(request),
                host,
            },
        })
    }
}
impl ConfiguredSessionTargets {
    pub(super) fn terminal_options(
        &self,
        request: &SessionRequest,
    ) -> Result<nocterm_agent_tools::headless::HeadlessTerminalOptions, CoordinatorError> {
        let entry = self
            .entries
            .iter()
            .find(|entry| {
                &entry.target == request.binding().target()
                    && &entry.authentication == request.binding().authentication()
            })
            .ok_or(CoordinatorError::Unsupported)?;
        let size = match &entry.connector {
            Connector::Local { size, .. } => *size,
            Connector::Ssh { request, .. } => request.size,
        };
        Ok(nocterm_agent_tools::headless::HeadlessTerminalOptions {
            size,
            charset: nocterm_session::Charset::Utf8,
        })
    }
    pub(crate) fn new(
        binding: JournalCatalogBinding,
        entries: Vec<ConfiguredSessionTarget>,
    ) -> Result<Self, CoordinatorError> {
        if entries.is_empty() || entries.len() > 128 {
            return Err(CoordinatorError::InvalidCommand);
        }
        for (index, entry) in entries.iter().enumerate() {
            if entries[..index]
                .iter()
                .any(|old| old.target == entry.target && old.authentication == entry.authentication)
            {
                return Err(CoordinatorError::Conflict);
            }
        }
        Ok(Self { binding, entries })
    }
    pub(crate) fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub(crate) fn open(
        &self,
        request: &SessionRequest,
        source: ConnectionSource,
        authorizer: Option<Arc<dyn SessionLoginAuthorizer>>,
        allocation: &AllocationLease,
    ) -> Result<Session, CoordinatorError> {
        if !self.binding.contains(&request.binding().run().stream()) || !source.is_restricted() {
            return Err(CoordinatorError::StaleSource);
        }
        if allocation.bytes() < TARGET_CAPTURE_BYTES || allocation.payloads() == 0 {
            return Err(CoordinatorError::Overloaded);
        }
        // Both actual transports currently open a PTY. Never silently promote a
        // connection-only/read-only request into shell execution.
        if request.startup() != SessionStartup::Terminal {
            return Err(CoordinatorError::Unsupported);
        }
        source.check().map_err(backend)?;
        let target = self
            .entries
            .iter()
            .find(|entry| {
                &entry.target == request.binding().target()
                    && &entry.authentication == request.binding().authentication()
            })
            .ok_or(CoordinatorError::Unsupported)?;
        match &target.connector {
            Connector::Local {
                local,
                launch,
                term,
                size,
            } => {
                if request.ssh_consent().is_some() || authorizer.is_some() {
                    return Err(CoordinatorError::Conflict);
                }
                Ok(local.open_shell(launch.clone(), term.clone(), *size, source))
            }
            Connector::Ssh {
                transport,
                request: connect,
                host,
            } => {
                let consent = request
                    .ssh_consent()
                    .ok_or(CoordinatorError::InvalidCommand)?;
                let attempt = SessionLoginAttempt::new(
                    consent.clone(),
                    connect.target.clone(),
                    host.clone(),
                    authorizer.ok_or(CoordinatorError::NeedsUnlock)?,
                )
                .map_err(backend)?;
                // open_login is the exclusive purpose-bound path. Its Auto DTO
                // field never enters legacy password/file/agent authentication.
                Ok(transport.open_login(
                    (**connect).clone(),
                    ConnectionOpenOptions::new(source),
                    attempt,
                ))
            }
        }
    }
}
fn validate_launch(
    launch: &ShellLaunch,
    term: &str,
    size: PtySize,
) -> Result<(), CoordinatorError> {
    launch.validate().map_err(backend)?;
    let strings = launch
        .program
        .as_ref()
        .map_or(0, String::len)
        .saturating_add(launch.cwd.as_ref().map_or(0, String::len))
        .saturating_add(launch.args.iter().map(String::len).sum::<usize>());
    if !launch.env.is_empty()
        || launch.args.len() > 64
        || strings > 16 * 1024
        || term.is_empty()
        || term.len() > 128
        || !term
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.+".contains(&byte))
        || !(2..=240).contains(&size.cols)
        || !(1..=120).contains(&size.rows)
    {
        return Err(CoordinatorError::InvalidCommand);
    }
    Ok(())
}
fn backend(error: impl std::error::Error + Send + Sync + 'static) -> CoordinatorError {
    CoordinatorError::Backend(SharedCause::new(error))
}

//! Trusted native targets; signed fingerprints select, never construct, I/O.
//!
//! An SSH target signs in with a device's one-use consent proof or, without one, with
//! the chat's in-memory key and the certificate a device issued for that bound server
//! (ADR 0137). Without an active certificate such a request waits for a device.
//!
//! A run may also name a bound server by the session access its binding defines (ADR
//! 0139). It selects the admitted server of the chat's binding with exactly that access,
//! then a connector with that server's host, port, user and host key: a configured SSH
//! target or a managed boot v3 target with a host key. It signs in only with the chat's
//! certificate.
use nocterm_cloud_model::{
    authority::{AuthenticationBinding, TargetBinding},
    coordinator::{CoordinatorError, SessionRequest, SessionStartup},
    identity::{Fingerprint, Timestamp},
    journal::JournalCatalogBinding,
    login::SessionLoginAuthorizer,
    ssh_access::{BoundLogin, ChatSshCredentials, SshEndpoint},
};
use nocterm_core::{SharedCause, allocation::AllocationLease};
use nocterm_local::ContainedLocal;
use nocterm_session::{
    Auth, ConnectRequest, ConnectionOpenOptions, ConnectionSource, PtySize, Session, ShellLaunch,
};
use nocterm_ssh::{CertificateLogin, SshTransport, proof::SessionLoginAttempt};
use nocterm_ssh_keys::PublicKeyText;
use std::sync::Arc;

pub(crate) const TARGET_CAPTURE_BYTES: usize = 64 * 1024;

pub(crate) struct ConfiguredSessionTargets {
    binding: JournalCatalogBinding,
    entries: Vec<ConfiguredSessionTarget>,
    /// Managed boot v3 targets with a host key; bound servers only.
    boot: Vec<BootConnector>,
    credentials: Option<Arc<dyn ChatSshCredentials>>,
}
/// A managed boot v3 target with a host key, reachable only as a bound server.
pub(crate) struct BootConnector {
    host_key: Fingerprint,
    transport: Arc<SshTransport>,
    request: Box<ConnectRequest>,
    host: PublicKeyText,
}
impl BootConnector {
    pub(crate) fn new(
        transport: Arc<SshTransport>,
        request: ConnectRequest,
        host: PublicKeyText,
    ) -> Result<Self, CoordinatorError> {
        validate_ssh(&request, &host)?;
        Ok(Self {
            host_key: Fingerprint::from_bytes(host.fingerprint_bytes().map_err(backend)?),
            transport,
            request: Box::new(request),
            host,
        })
    }
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
        validate_ssh(&request, &host)?;
        if request.target.user != target.principal().as_str()
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
        now: Timestamp,
    ) -> Result<nocterm_agent_tools::headless::HeadlessTerminalOptions, CoordinatorError> {
        let size = match self.entry(request) {
            Some(entry) => match &entry.connector {
                Connector::Local { size, .. } => *size,
                Connector::Ssh { request, .. } => request.size,
            },
            None => {
                let bound = self.bound(request, now)?;
                self.connector(&bound)?.1.size
            }
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
        Ok(Self {
            binding,
            entries,
            boot: Vec::new(),
            credentials: None,
        })
    }
    /// Lets bound servers that are managed boot targets be signed in to (ADR 0139).
    pub(crate) fn with_boot_targets(mut self, boot: Vec<BootConnector>) -> Self {
        self.boot = boot;
        self
    }
    /// Lets SSH targets sign in with the chats' certificates (ADR 0137).
    pub(crate) fn with_credentials(mut self, credentials: Arc<dyn ChatSshCredentials>) -> Self {
        self.credentials = Some(credentials);
        self
    }
    /// The targets that run inside this runner's own containment. Every other
    /// target is a remote machine the runner does not contain.
    pub(crate) fn contained_targets(&self) -> Vec<TargetBinding> {
        self.entries
            .iter()
            .filter(|entry| matches!(entry.connector, Connector::Local { .. }))
            .map(|entry| entry.target.clone())
            .collect()
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
        now: Timestamp,
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
        let Some(target) = self.entry(request) else {
            return self.open_bound(request, source, authorizer.is_some(), now);
        };
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
                let Some(consent) = request.ssh_consent() else {
                    if authorizer.is_some() {
                        return Err(CoordinatorError::Conflict);
                    }
                    let login = self.certificate(request, &target.target, connect, host, now)?;
                    return Ok(transport.open_certificate(
                        (**connect).clone(),
                        ConnectionOpenOptions::new(source),
                        login,
                    ));
                };
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
impl ConfiguredSessionTargets {
    fn entry(&self, request: &SessionRequest) -> Option<&ConfiguredSessionTarget> {
        self.entries.iter().find(|entry| {
            &entry.target == request.binding().target()
                && &entry.authentication == request.binding().authentication()
        })
    }
    /// The bound server of the chat's binding that the request's access selects.
    fn bound(
        &self,
        request: &SessionRequest,
        now: Timestamp,
    ) -> Result<BoundLogin, CoordinatorError> {
        let binding = request.binding();
        self.credentials
            .as_ref()
            .and_then(|credentials| {
                credentials.bound(
                    &binding.run().stream(),
                    binding.target(),
                    binding.authentication(),
                    now,
                )
            })
            .ok_or(CoordinatorError::Unsupported)
    }
    /// A configured SSH target or boot target with the bound server's endpoint and host key.
    fn connector(
        &self,
        bound: &BoundLogin,
    ) -> Result<(&Arc<SshTransport>, &ConnectRequest, &PublicKeyText), CoordinatorError> {
        let same = |request: &ConnectRequest| {
            request.target.host == bound.endpoint.host
                && request.target.port == bound.endpoint.port
                && request.target.user == bound.endpoint.user
        };
        let configured = self
            .entries
            .iter()
            .find_map(|entry| match &entry.connector {
                Connector::Ssh {
                    transport,
                    request,
                    host,
                } if same(request) && entry.target.host() == bound.host_key => {
                    Some((transport, &**request, host))
                }
                _ => None,
            });
        configured
            .or_else(|| {
                self.boot
                    .iter()
                    .find(|boot| same(&boot.request) && boot.host_key == bound.host_key)
                    .map(|boot| (&boot.transport, &*boot.request, &boot.host))
            })
            .ok_or(CoordinatorError::Unsupported)
    }
    /// Signs in to a bound server with the chat's certificate; waits for a device without one.
    fn open_bound(
        &self,
        request: &SessionRequest,
        source: ConnectionSource,
        authorized: bool,
        now: Timestamp,
    ) -> Result<Session, CoordinatorError> {
        let bound = self.bound(request, now)?;
        let (transport, connect, host) = self.connector(&bound)?;
        // A bound server signs in with the chat's certificate only, never with a consent.
        if request.ssh_consent().is_some() || authorized {
            return Err(CoordinatorError::Conflict);
        }
        let credential = bound.credential.ok_or(CoordinatorError::NeedsUnlock)?;
        let login = CertificateLogin::new(
            connect.target.clone(),
            host,
            credential.seed.expose(),
            credential.certificate.as_bytes(),
        )
        .map_err(backend)?;
        Ok(transport.open_certificate(connect.clone(), ConnectionOpenOptions::new(source), login))
    }
    /// The chat's certificate login for this configured target, when one is active.
    fn certificate(
        &self,
        request: &SessionRequest,
        target: &TargetBinding,
        connect: &ConnectRequest,
        host: &PublicKeyText,
        now: Timestamp,
    ) -> Result<CertificateLogin, CoordinatorError> {
        let endpoint = SshEndpoint {
            host: connect.target.host.clone(),
            port: connect.target.port,
            user: connect.target.user.clone(),
        };
        let credential = self
            .credentials
            .as_ref()
            .and_then(|credentials| {
                credentials.credential(
                    &request.binding().run().stream(),
                    &endpoint,
                    &target.host(),
                    now,
                )
            })
            .ok_or(CoordinatorError::NeedsUnlock)?;
        CertificateLogin::new(
            connect.target.clone(),
            host,
            credential.seed.expose(),
            credential.certificate.as_bytes(),
        )
        .map_err(backend)
    }
}
fn validate_ssh(request: &ConnectRequest, host: &PublicKeyText) -> Result<(), CoordinatorError> {
    validate_launch(&request.launch, &request.term, request.size)?;
    if request.proxy != nocterm_session::ProxyConfig::Direct
        || !request.jumps.is_empty()
        || request.auth != Auth::Auto
        || request.target.host.is_empty()
        || request.target.host.len() > 253
        || request.target.port == 0
        || request.connect_timeout.is_zero()
        || request.connect_timeout > std::time::Duration::from_secs(60)
        || host.as_str().len() > 2048
    {
        return Err(CoordinatorError::InvalidCommand);
    }
    Ok(())
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

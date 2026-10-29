//! Original immutable run, target, session and grant coordinates.
use crate::identity::{
    AccountScope, CloudChatId, Fingerprint, GrantEpoch, GrantId, NamespaceId, RemoteSessionId,
    RunEpoch, RunId, RunnerId, RunnerKeyEpoch, SessionEpoch,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunBinding {
    scope: AccountScope,
    runner: RunnerId,
    runner_epoch: RunnerKeyEpoch,
    chat: CloudChatId,
    run: RunId,
    epoch: RunEpoch,
}
impl RunBinding {
    pub fn new(
        scope: AccountScope,
        runner: RunnerId,
        runner_epoch: RunnerKeyEpoch,
        chat: CloudChatId,
        run: RunId,
        epoch: RunEpoch,
    ) -> Self {
        Self {
            scope,
            runner,
            runner_epoch,
            chat,
            run,
            epoch,
        }
    }
    pub fn identity(&self) -> ChatIdentity {
        ChatIdentity::new(self.scope.clone(), self.chat)
    }
    pub fn scope(&self) -> &AccountScope {
        &self.scope
    }
    pub fn runner(&self) -> RunnerId {
        self.runner
    }
    pub fn runner_epoch(&self) -> RunnerKeyEpoch {
        self.runner_epoch
    }
    pub fn chat(&self) -> CloudChatId {
        self.chat
    }
    pub fn run(&self) -> RunId {
        self.run
    }
    pub fn epoch(&self) -> RunEpoch {
        self.epoch
    }
    pub fn with_epoch(&self, epoch: RunEpoch) -> Self {
        Self {
            epoch,
            ..self.clone()
        }
    }
    pub fn same_chat(&self, other: &Self) -> bool {
        self.scope == other.scope && self.chat == other.chat
    }
    pub fn same_run(&self, other: &Self) -> bool {
        self.same_chat(other)
            && self.runner == other.runner
            && self.runner_epoch == other.runner_epoch
            && self.run == other.run
    }
    pub fn stream(&self) -> StreamBinding {
        StreamBinding::new(
            self.scope.clone(),
            self.runner,
            self.runner_epoch,
            self.chat,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamBinding {
    scope: AccountScope,
    runner: RunnerId,
    runner_epoch: RunnerKeyEpoch,
    chat: CloudChatId,
}
impl StreamBinding {
    pub fn new(
        scope: AccountScope,
        runner: RunnerId,
        runner_epoch: RunnerKeyEpoch,
        chat: CloudChatId,
    ) -> Self {
        Self {
            scope,
            runner,
            runner_epoch,
            chat,
        }
    }
    pub fn identity(&self) -> ChatIdentity {
        ChatIdentity::new(self.scope.clone(), self.chat)
    }
    pub fn scope(&self) -> &AccountScope {
        &self.scope
    }
    pub fn runner(&self) -> RunnerId {
        self.runner
    }
    pub fn runner_epoch(&self) -> RunnerKeyEpoch {
        self.runner_epoch
    }
    pub fn chat(&self) -> CloudChatId {
        self.chat
    }
}

/// Canonical target observations supplied by the transport/workspace owner.
/// Workspace identity is not a filesystem sandbox. Profile names/aliases are absent.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetBinding {
    host: Fingerprint,
    route: Fingerprint,
    principal: NamespaceId,
    workspace: Fingerprint,
}
impl TargetBinding {
    pub fn new(
        host: Fingerprint,
        route: Fingerprint,
        principal: NamespaceId,
        canonical_workspace: Fingerprint,
    ) -> Self {
        Self {
            host,
            route,
            principal,
            workspace: canonical_workspace,
        }
    }
    /// Conflict lane only. Execution still requires the exact original route
    /// and session; an alias never relaxes its authority coordinates.
    pub fn same_mutation_lane(&self, other: &Self) -> bool {
        self.host == other.host
            && self.principal == other.principal
            && self.workspace == other.workspace
    }
    pub fn host(&self) -> Fingerprint {
        self.host
    }
    pub fn route(&self) -> Fingerprint {
        self.route
    }
    pub fn principal(&self) -> &NamespaceId {
        &self.principal
    }
    pub fn workspace(&self) -> Fingerprint {
        self.workspace
    }
}

/// Describes the exact runner-owned SSH session; it does not open or authenticate it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionReceipt {
    id: RemoteSessionId,
    epoch: SessionEpoch,
    target: TargetBinding,
    authentication: AuthenticationBinding,
}
impl SessionReceipt {
    pub fn new(
        id: RemoteSessionId,
        epoch: SessionEpoch,
        target: TargetBinding,
        authentication: AuthenticationBinding,
    ) -> Self {
        Self {
            id,
            epoch,
            target,
            authentication,
        }
    }
    pub fn authentication(&self) -> &AuthenticationBinding {
        &self.authentication
    }
    pub fn id(&self) -> RemoteSessionId {
        self.id
    }
    pub fn epoch(&self) -> SessionEpoch {
        self.epoch
    }
    pub fn target(&self) -> &TargetBinding {
        &self.target
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionAuthority {
    run: RunBinding,
    session: SessionReceipt,
    grant: GrantId,
    grant_epoch: GrantEpoch,
}
impl ExecutionAuthority {
    pub fn new(
        run: RunBinding,
        session: SessionReceipt,
        grant: GrantId,
        grant_epoch: GrantEpoch,
    ) -> Self {
        Self {
            run,
            session,
            grant,
            grant_epoch,
        }
    }
    pub fn run(&self) -> &RunBinding {
        &self.run
    }
    pub fn session(&self) -> &SessionReceipt {
        &self.session
    }
    pub fn grant(&self) -> GrantId {
        self.grant
    }
    pub fn grant_epoch(&self) -> GrantEpoch {
        self.grant_epoch
    }
    pub fn require_same(&self, expected: &Self) -> Result<(), AuthorityError> {
        if self.run.scope() != expected.run.scope() {
            return Err(AuthorityError::WrongScope);
        }
        if self.run != expected.run {
            return Err(AuthorityError::StaleRun);
        }
        if self.session.target != expected.session.target {
            return Err(AuthorityError::WrongTarget);
        }
        if self.session != expected.session {
            return Err(AuthorityError::StaleSession);
        }
        if self.grant != expected.grant || self.grant_epoch != expected.grant_epoch {
            return Err(AuthorityError::StaleGrant);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AuthorityError {
    #[error("the original hub/account scope does not match")]
    WrongScope,
    #[error("the original run or run epoch does not match")]
    StaleRun,
    #[error("the original pinned target does not match")]
    WrongTarget,
    #[error("the original session or session epoch does not match")]
    StaleSession,
    #[error("the original grant or grant epoch does not match")]
    StaleGrant,
}

/// Stable storage uniqueness is account plus chat, independent of runner assignment.
/// A journal must reject a different runner binding for an existing identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChatIdentity {
    scope: AccountScope,
    chat: CloudChatId,
}
impl ChatIdentity {
    pub fn new(scope: AccountScope, chat: CloudChatId) -> Self {
        Self { scope, chat }
    }
    pub fn scope(&self) -> &AccountScope {
        &self.scope
    }
    pub fn chat(&self) -> CloudChatId {
        self.chat
    }
}

/// Selected SSH authentication facts supplied by its credential/session owner.
/// A changed selected key or credential revision invalidates the old receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticationBinding {
    key: Fingerprint,
    credential_revision: Fingerprint,
}
impl AuthenticationBinding {
    pub fn new(key: Fingerprint, credential_revision: Fingerprint) -> Self {
        Self {
            key,
            credential_revision,
        }
    }
    pub fn key(&self) -> Fingerprint {
        self.key
    }
    pub fn credential_revision(&self) -> Fingerprint {
        self.credential_revision
    }
}

/// Captured actual credentials. Token digest is attribution, never a stable auth-session ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandIssuer {
    device: NamespaceId,
    key: Fingerprint,
    invocation: Fingerprint,
}
impl CommandIssuer {
    pub fn new(device: NamespaceId, key: Fingerprint, invocation: Fingerprint) -> Self {
        Self {
            device,
            key,
            invocation,
        }
    }
    pub fn device(&self) -> &NamespaceId {
        &self.device
    }
    /// Exact original Ed25519 device public key bytes, not SHA256(public key).
    /// The Fingerprint wrapper stores bytes here without hashing them.
    pub fn key(&self) -> Fingerprint {
        self.key
    }
    /// SHA256 attribution of the original access token, not a stable session ID.
    pub fn invocation(&self) -> Fingerprint {
        self.invocation
    }
}

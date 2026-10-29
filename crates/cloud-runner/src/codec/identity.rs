use nocterm_cloud_model::{authority::*, identity::*, journal::JournalError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Scope {
    instance: String,
    authority: [u8; 32],
    account: String,
    key: [u8; 32],
}
impl From<&AccountScope> for Scope {
    fn from(value: &AccountScope) -> Self {
        Self {
            instance: value.hub().instance().as_str().into(),
            authority: *value.hub().authority().as_bytes(),
            account: value.account().id().as_str().into(),
            key: *value.account().key().as_bytes(),
        }
    }
}
impl Scope {
    pub(crate) fn restore(self) -> Result<AccountScope, JournalError> {
        Ok(AccountScope::new(
            HubIdentity::new(namespace(self.instance)?, fp(self.authority)),
            AccountIdentity::new(namespace(self.account)?, fp(self.key)),
        ))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Stream {
    scope: Scope,
    runner: [u8; 16],
    epoch: u64,
    chat: [u8; 16],
}
impl From<&StreamBinding> for Stream {
    fn from(value: &StreamBinding) -> Self {
        Self {
            scope: value.scope().into(),
            runner: *value.runner().as_bytes(),
            epoch: value.runner_epoch().get(),
            chat: *value.chat().as_bytes(),
        }
    }
}
impl Stream {
    pub(crate) fn restore(self) -> Result<StreamBinding, JournalError> {
        Ok(StreamBinding::new(
            self.scope.restore()?,
            RunnerId::from_bytes(self.runner),
            RunnerKeyEpoch::new(self.epoch)?,
            CloudChatId::from_bytes(self.chat),
        ))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Run {
    stream: Stream,
    id: [u8; 16],
    epoch: u64,
}
impl From<&RunBinding> for Run {
    fn from(value: &RunBinding) -> Self {
        Self {
            stream: (&value.stream()).into(),
            id: *value.run().as_bytes(),
            epoch: value.epoch().get(),
        }
    }
}
impl Run {
    pub(crate) fn restore(self) -> Result<RunBinding, JournalError> {
        let stream = self.stream.restore()?;
        Ok(RunBinding::new(
            stream.scope().clone(),
            stream.runner(),
            stream.runner_epoch(),
            stream.chat(),
            RunId::from_bytes(self.id),
            RunEpoch::new(self.epoch)?,
        ))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Authority {
    run: Run,
    session: [u8; 16],
    session_epoch: u64,
    host: [u8; 32],
    route: [u8; 32],
    principal: String,
    workspace: [u8; 32],
    auth_key: [u8; 32],
    credential: [u8; 32],
    grant: [u8; 16],
    grant_epoch: u64,
}
impl From<&ExecutionAuthority> for Authority {
    fn from(value: &ExecutionAuthority) -> Self {
        let session = value.session();
        let target = session.target();
        Self {
            run: value.run().into(),
            session: *session.id().as_bytes(),
            session_epoch: session.epoch().get(),
            host: *target.host().as_bytes(),
            route: *target.route().as_bytes(),
            principal: target.principal().as_str().into(),
            workspace: *target.workspace().as_bytes(),
            auth_key: *session.authentication().key().as_bytes(),
            credential: *session.authentication().credential_revision().as_bytes(),
            grant: *value.grant().as_bytes(),
            grant_epoch: value.grant_epoch().get(),
        }
    }
}
impl Authority {
    pub(crate) fn restore(self) -> Result<ExecutionAuthority, JournalError> {
        let target = TargetBinding::new(
            fp(self.host),
            fp(self.route),
            namespace(self.principal)?,
            fp(self.workspace),
        );
        let auth = AuthenticationBinding::new(fp(self.auth_key), fp(self.credential));
        Ok(ExecutionAuthority::new(
            self.run.restore()?,
            SessionReceipt::new(
                RemoteSessionId::from_bytes(self.session),
                SessionEpoch::new(self.session_epoch)?,
                target,
                auth,
            ),
            GrantId::from_bytes(self.grant),
            GrantEpoch::new(self.grant_epoch)?,
        ))
    }
}

pub(crate) fn namespace(value: String) -> Result<NamespaceId, JournalError> {
    NamespaceId::new(value).map_err(|_| JournalError::Corrupt)
}
pub(crate) fn fp(bytes: [u8; 32]) -> Fingerprint {
    Fingerprint::from_bytes(bytes)
}

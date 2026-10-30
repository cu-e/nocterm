//! Native accepted-request format; public canonical bytes stay contract-owned.
use super::format::Destination;
use crate::codec::identity::Run;
use nocterm_cloud_model::{
    coordinator::{CoordinatorError, SessionAttempt, SessionRequest, SessionStartup},
    grant::GrantWindow,
    identity::{AttemptId, Fingerprint, Timestamp},
    journal::JournalError,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SessionStart {
    run: Run,
    destination: Destination,
    attempt: [u8; 16],
    nonce: [u8; 32],
    issued: u64,
    expires: u64,
    terminal: bool,
    user_key: Option<String>,
}
impl From<&SessionRequest> for SessionStart {
    fn from(request: &SessionRequest) -> Self {
        let binding = request.binding();
        let destination = Destination::selection(binding.target(), binding.authentication(), None);
        Self {
            run: binding.run().into(),
            destination,
            attempt: *binding.attempt().as_bytes(),
            nonce: *binding.nonce().as_bytes(),
            issued: binding.window().issued().as_millis(),
            expires: binding.window().expires().as_millis(),
            terminal: request.startup() == SessionStartup::Terminal,
            user_key: request
                .ssh_consent()
                .map(|consent| consent.user_key().as_str().to_owned()),
        }
    }
}
impl SessionStart {
    pub(super) fn restore(self) -> Result<SessionRequest, CoordinatorError> {
        let destination = self.destination.restore()?;
        if destination.session().is_some() {
            return Err(JournalError::Corrupt.into());
        }
        let binding = SessionAttempt::new(
            self.run.restore()?,
            destination.target().clone(),
            destination.authentication().clone(),
            AttemptId::from_bytes(self.attempt),
            Fingerprint::from_bytes(self.nonce),
            GrantWindow::new(
                Timestamp::from_millis(self.issued),
                Timestamp::from_millis(self.expires),
            )?,
        )?;
        let startup = if self.terminal {
            SessionStartup::Terminal
        } else {
            SessionStartup::ConnectOnly
        };
        match self.user_key {
            Some(key) => SessionRequest::ssh_key(binding, startup, &key),
            None => Ok(SessionRequest::local(binding, startup)),
        }
    }
}

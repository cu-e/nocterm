//! A captured private-file origin fences a delayed explicit login publication.
use super::{CredentialError, CredentialFile, CredentialLoginId, Device};
use nocterm_hub_client::{HubClient, Session};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct CredentialOrigin {
    public_key: [u8; 32],
    login: Option<CredentialLoginId>,
    subject: Option<(String, String)>,
}
impl CredentialOrigin {
    pub(super) fn of(device: &Device) -> Self {
        Self {
            public_key: device.key.public_key(),
            login: device.login,
            subject: device
                .session
                .as_ref()
                .map(|session| (session.account_id.clone(), session.device_id.clone())),
        }
    }
    pub(super) fn matches(&self, device: &Device) -> bool {
        self.public_key == device.key.public_key()
            && self.login == device.login
            && self
                .subject
                .as_ref()
                .map(|(account, device)| (account.as_str(), device.as_str()))
                == device
                    .session
                    .as_ref()
                    .map(|session| (session.account_id.as_str(), session.device_id.as_str()))
    }
    pub(crate) fn public_key(&self) -> &[u8; 32] {
        &self.public_key
    }
}

pub(crate) struct LoginPublication<'a> {
    file: &'a CredentialFile,
    session: &'a Session,
    origin: &'a CredentialOrigin,
}
impl<'a> LoginPublication<'a> {
    pub(crate) fn new(
        file: &'a CredentialFile,
        session: &'a Session,
        origin: &'a CredentialOrigin,
    ) -> Self {
        Self {
            file,
            session,
            origin,
        }
    }
    pub(crate) fn session(&self) -> &Session {
        self.session
    }
    pub(crate) fn save(&self) -> Result<CredentialLoginId, CredentialError> {
        self.file.save_session_expected(self.session, self.origin)
    }
    pub(crate) fn bind(&self, client: &HubClient, login: CredentialLoginId) {
        client.persist_with(Arc::new(self.file.clone().bound(self.session, Some(login))));
    }
}

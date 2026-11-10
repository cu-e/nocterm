//! A client's token store can refresh its original account/device only.
use std::{any::Any, io};

use nocterm_hub_client::{Session, SessionStore};
use zeroize::Zeroize as _;

use super::{CredentialFile, CredentialLoginId};

#[derive(Debug)]
pub(crate) struct BoundSessionStore {
    file: CredentialFile,
    account: String,
    device: String,
    login: Option<CredentialLoginId>,
}

#[derive(Debug, thiserror::Error)]
#[error("the stored login no longer belongs to this client")]
struct ReplacedLogin;

impl BoundSessionStore {
    pub(super) fn new(
        file: CredentialFile,
        session: &Session,
        login: Option<CredentialLoginId>,
    ) -> Self {
        Self {
            file,
            account: session.account_id.clone(),
            device: session.device_id.clone(),
            login,
        }
    }

    fn matches(&self, session: &Session) -> bool {
        session.account_id == self.account && session.device_id == self.device
    }

    fn current(&self) -> io::Result<Session> {
        let current = self.file.load().map_err(super::into_io)?;
        let same_login = current
            .as_ref()
            .is_some_and(|stored| stored.login == self.login);
        let Some(mut session) = current.and_then(|stored| stored.session) else {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                ReplacedLogin,
            ));
        };
        if !same_login || !self.matches(&session) {
            session.access_token.zeroize();
            session.refresh_token.zeroize();
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                ReplacedLogin,
            ));
        }
        Ok(session)
    }
}

impl SessionStore for BoundSessionStore {
    fn lock(&self) -> io::Result<Box<dyn Any + Send>> {
        SessionStore::lock(&self.file)
    }

    fn load(&self) -> io::Result<Option<Session>> {
        self.current().map(Some)
    }

    fn save(&self, session: &Session) -> io::Result<()> {
        if !self.matches(session) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                ReplacedLogin,
            ));
        }
        // SDK owns the file lock through load, remote refresh and save. A new
        // login uses that same lock, so this check cannot race replacement.
        let mut current = self.current()?;
        current.access_token.zeroize();
        current.refresh_token.zeroize();
        SessionStore::save(&self.file, session)
    }
}

#[cfg(test)]
mod tests;

//! The private credential file of this device for each hub.
//!
//! One file per hub, `<host>_<port>.toml` in a mode-0700 directory, holds the device key seed
//! and the tokens. A sibling `.lock` file serializes token rotation between processes.

use std::any::Any;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use nocterm_core::fs::{
    BoundedReadError, create_private, read_regular_bounded, write_private_atomic,
};
use nocterm_hub_client::{Session, SessionStore};
use nocterm_hubs_model::HubAddress;
use nocterm_keys::DeviceKey;
use serde::{Deserialize, Serialize};
use tokio::runtime::{Handle, RuntimeFlavor};

mod bound;
mod publication;
mod seed;
use bound::BoundSessionStore;
pub(crate) use publication::{CredentialOrigin, LoginPublication};

/// A credential file is a few hundred bytes; anything larger is not ours.
const MAX_FILE_BYTES: usize = 16 * 1024;
/// Keeps `<name>.toml.lock` below the usual 255-byte file name limit.
pub(crate) const MAX_NAME_BYTES: usize = 200;

/// Why the credentials of a hub could not be read or kept.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CredentialError {
    #[error("the credential file cannot be read")]
    Read(#[from] BoundedReadError),
    #[error("the credential file cannot be written")]
    Write(#[from] nocterm_core::fs::FileError),
    #[error("the credential directory or lock cannot be prepared")]
    Prepare(#[source] io::Error),
    #[error("the credential file is not valid")]
    Format(#[from] toml::de::Error),
    #[error("the credential file cannot be encoded")]
    Encode(#[from] toml::ser::Error),
    #[error("the credential file holds no valid device key")]
    Seed(#[from] seed::SeedError),
    #[error("the credential file belongs to another hub")]
    OtherHub,
    #[error("the local login identity cannot be generated")]
    LoginIdentity(#[source] nocterm_keys::KeyError),
    #[error("the original local login file was replaced")]
    StaleLogin,
    #[error("the hub address is too long for a file name")]
    NameTooLong,
}

/// Where every hub's credential file lives.
#[derive(Clone, Debug)]
pub(crate) struct CredentialDir {
    root: PathBuf,
}

impl CredentialDir {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn file(&self, address: &HubAddress) -> Result<CredentialFile, CredentialError> {
        let name = file_stem(address);
        if name.len() > MAX_NAME_BYTES {
            return Err(CredentialError::NameTooLong);
        }
        Ok(CredentialFile {
            path: self.root.join(format!("{name}.toml")),
            lock_path: self.root.join(format!("{name}.toml.lock")),
            address: address.clone(),
        })
    }
}

/// `host_port`; `:` becomes `_`, which no host name contains, so names are unique.
pub(crate) fn file_stem(address: &HubAddress) -> String {
    // Also names the account cache beside the credential file.
    format!("{}_{}", address.host(), address.port()).replace(':', "_")
}

/// Identifies one explicit native login, independently of account/device reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CredentialLoginId([u8; 16]);

/// What the file holds, as stored.
#[derive(Serialize, Deserialize)]
struct Stored {
    address: String,
    seed: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    login: Option<CredentialLoginId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<Session>,
}

/// The device key and tokens read from a credential file.
pub(crate) struct Device {
    pub(crate) key: DeviceKey,
    pub(crate) session: Option<Session>,
    pub(crate) login: Option<CredentialLoginId>,
}
impl Device {
    pub(crate) fn origin(&self) -> CredentialOrigin {
        CredentialOrigin::of(self)
    }
    pub(crate) fn wipe_session(&mut self) {
        use zeroize::Zeroize as _;
        if let Some(session) = &mut self.session {
            session.access_token.zeroize();
            session.refresh_token.zeroize();
        }
    }
}

/// The credential file of one hub. Also the [`SessionStore`] its client rotates tokens in.
#[derive(Clone, Debug)]
pub(crate) struct CredentialFile {
    path: PathBuf,
    lock_path: PathBuf,
    address: HubAddress,
}

impl CredentialFile {
    /// Pins token adoption and refresh publication to this original login subject.
    pub(crate) fn bound(
        self,
        session: &Session,
        login: Option<CredentialLoginId>,
    ) -> BoundSessionStore {
        BoundSessionStore::new(self, session, login)
    }

    /// The stored device; `None` when nothing was ever kept for the hub.
    pub(crate) fn load(&self) -> Result<Option<Device>, CredentialError> {
        let bytes = match read_regular_bounded(&self.path, MAX_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.io_kind() == Some(io::ErrorKind::NotFound) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let stored = self.parse(&bytes)?;
        Ok(Some(Device {
            key: DeviceKey::from_seed(&seed::decode(&stored.seed)?),
            session: stored.session,
            login: stored.login,
        }))
    }

    fn parse(&self, bytes: &[u8]) -> Result<Stored, CredentialError> {
        let text = String::from_utf8_lossy(bytes);
        let stored: Stored = toml::from_str(&text)?;
        if stored.address != self.address.to_string() {
            return Err(CredentialError::OtherHub);
        }
        Ok(stored)
    }

    /// The stored device, or a new key kept without tokens. The flag says it was created.
    #[cfg(test)]
    pub(crate) fn load_or_create(&self) -> Result<(Device, bool), CredentialError> {
        let _lock = self.lock_exclusive().map_err(CredentialError::Prepare)?;
        if let Some(device) = self.load()? {
            return Ok((device, false));
        }
        let key = DeviceKey::generate()
            .map_err(|error| CredentialError::Prepare(io::Error::other(error)))?;
        self.write(&key, None, None)?;
        Ok((
            Device {
                key,
                session: None,
                login: None,
            },
            true,
        ))
    }

    /// Publishes a new explicit login, even when the subject is unchanged.
    #[cfg(test)]
    pub(crate) fn save_session(
        &self,
        session: &Session,
    ) -> Result<CredentialLoginId, CredentialError> {
        self.save_session_inner(session, None)
    }

    fn save_session_expected(
        &self,
        session: &Session,
        origin: &CredentialOrigin,
    ) -> Result<CredentialLoginId, CredentialError> {
        self.save_session_inner(session, Some(origin))
    }

    fn save_session_inner(
        &self,
        session: &Session,
        origin: Option<&CredentialOrigin>,
    ) -> Result<CredentialLoginId, CredentialError> {
        let login = CredentialLoginId(
            nocterm_keys::random_bytes().map_err(CredentialError::LoginIdentity)?,
        );
        let _lock = self.lock_exclusive().map_err(CredentialError::Prepare)?;
        let mut device = self
            .load()?
            .ok_or_else(|| CredentialError::Prepare(io::Error::from(io::ErrorKind::NotFound)))?;
        let current = origin.is_none_or(|origin| origin.matches(&device));
        device.wipe_session();
        if !current {
            return Err(CredentialError::StaleLogin);
        }
        self.write(&device.key, Some(session), Some(login))?;
        Ok(login)
    }

    /// Establishes only the captured original device file before native login I/O.
    pub(crate) fn prepare_login(
        &self,
        device: &Device,
        absent: bool,
    ) -> Result<(), CredentialError> {
        let _lock = self.lock_exclusive().map_err(CredentialError::Prepare)?;
        match self.load()? {
            None if absent => self.write(&device.key, None, None),
            Some(mut current) if !absent => {
                let matches = device.origin().matches(&current);
                current.wipe_session();
                if matches {
                    Ok(())
                } else {
                    Err(CredentialError::StaleLogin)
                }
            }
            Some(mut current) => {
                current.wipe_session();
                Err(CredentialError::StaleLogin)
            }
            None => Err(CredentialError::StaleLogin),
        }
    }

    /// Forgets the tokens but keeps the key, so the device signs in again as itself.
    #[cfg(test)]
    pub(crate) fn clear_session(&self) -> Result<(), CredentialError> {
        let _lock = self.lock_exclusive().map_err(CredentialError::Prepare)?;
        self.replace_session(None)
    }

    /// Replaces the tokens for a caller that holds the lock already: the client of the
    /// contract does while it rotates. A second lock would wait for itself.
    fn save_locked(&self, session: &Session) -> Result<(), CredentialError> {
        self.replace_session(Some(session))
    }

    fn replace_session(&self, session: Option<&Session>) -> Result<(), CredentialError> {
        let device = self
            .load()?
            .ok_or_else(|| CredentialError::Prepare(io::Error::from(io::ErrorKind::NotFound)))?;
        self.write(&device.key, session, device.login)
    }

    fn write(
        &self,
        key: &DeviceKey,
        session: Option<&Session>,
        login: Option<CredentialLoginId>,
    ) -> Result<(), CredentialError> {
        self.prepare_directory().map_err(CredentialError::Prepare)?;
        let stored = Stored {
            address: self.address.to_string(),
            seed: seed::encode(&key.seed()),
            session: session.cloned(),
            login,
        };
        let text = toml::to_string(&stored)?;
        write_private_atomic(&self.path, text.as_bytes())?;
        Ok(())
    }

    /// Checks a captured login while excluding token/file replacement.
    pub(crate) fn matches_login(
        &self,
        account: &str,
        device: &str,
        login: Option<CredentialLoginId>,
    ) -> Result<bool, CredentialError> {
        let _lock = self.lock_exclusive().map_err(CredentialError::Prepare)?;
        self.matches_login_locked(account, device, login)
    }

    fn matches_login_locked(
        &self,
        account: &str,
        device: &str,
        login: Option<CredentialLoginId>,
    ) -> Result<bool, CredentialError> {
        use zeroize::Zeroize as _;
        Ok(self.load()?.is_some_and(|stored| {
            stored.session.is_some_and(|mut session| {
                let matches = stored.login == login
                    && session.account_id == account
                    && session.device_id == device;
                session.access_token.zeroize();
                session.refresh_token.zeroize();
                matches
            })
        }))
    }

    /// Retires only the original local login under the shared file lock.
    #[cfg(test)]
    pub(crate) fn erase_if(
        &self,
        account: &str,
        device: &str,
        login: Option<CredentialLoginId>,
    ) -> Result<(), CredentialError> {
        self.retire_if(account, device, login, true, || {})
            .map(|_| ())
    }

    /// The existing file-generation check also fences original cache retirement.
    /// Callers hold Pool admission; the action must not reacquire this file lock.
    pub(crate) fn retire_if(
        &self,
        account: &str,
        device: &str,
        login: Option<CredentialLoginId>,
        erase: bool,
        retire: impl FnOnce(),
    ) -> Result<bool, CredentialError> {
        let _lock = self.lock_exclusive().map_err(CredentialError::Prepare)?;
        let matches = self.matches_login_locked(account, device, login)?;
        if matches {
            retire();
            if erase {
                self.remove_locked().map_err(CredentialError::Prepare)?;
            } else {
                self.replace_session(None)?;
            }
        }
        Ok(matches)
    }

    /// A failed first sign-in owns only its still unpublished original device file.
    pub(crate) fn erase_unpublished(
        &self,
        expected_public_key: &[u8; 32],
    ) -> Result<(), CredentialError> {
        use zeroize::Zeroize as _;
        let _lock = self.lock_exclusive().map_err(CredentialError::Prepare)?;
        let Some(mut stored) = self.load()? else {
            return Ok(());
        };
        let unpublished = stored.login.is_none()
            && stored.session.is_none()
            && stored.key.public_key() == *expected_public_key;
        if let Some(session) = &mut stored.session {
            session.access_token.zeroize();
            session.refresh_token.zeroize();
        }
        if unpublished {
            self.remove_locked().map_err(CredentialError::Prepare)?;
        }
        Ok(())
    }

    pub(crate) fn retire_origin(
        &self,
        origin: &CredentialOrigin,
        retire: impl FnOnce(),
    ) -> Result<bool, CredentialError> {
        let _lock = self.lock_exclusive().map_err(CredentialError::Prepare)?;
        let Some(mut current) = self.load()? else {
            return Ok(false);
        };
        let matches = origin.matches(&current);
        current.wipe_session();
        if matches {
            retire();
            self.remove_locked().map_err(CredentialError::Prepare)?;
        }
        Ok(matches)
    }

    pub(crate) fn while_absent(&self, retire: impl FnOnce()) -> Result<bool, CredentialError> {
        let _lock = self.lock_exclusive().map_err(CredentialError::Prepare)?;
        if let Some(mut current) = self.load()? {
            current.wipe_session();
            return Ok(false);
        }
        retire();
        Ok(true)
    }

    /// Removes the credentials of the hub; no file is not an error.
    pub(crate) fn erase(&self) -> io::Result<()> {
        let _lock = self.lock_exclusive()?;
        self.remove_locked()
    }

    fn remove_locked(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    fn prepare_directory(&self) -> io::Result<()> {
        match self.path.parent() {
            Some(parent) => prepare_private_directory(parent),
            None => Ok(()),
        }
    }

    /// Opens the lock file, creating it private, and waits for the exclusive lock. Waiting
    /// does not hold up the worker of the runtime it runs on.
    fn lock_exclusive(&self) -> io::Result<File> {
        self.prepare_directory()?;
        let file = match create_private(&self.lock_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                open_existing(&self.lock_path)?
            }
            Err(error) => return Err(error.into_io_error()),
        };
        off_worker(|| file.lock())?;
        Ok(file)
    }
}

/// Creates `directory` and its parents, private to the user where the platform has modes.
pub(crate) fn prepare_private_directory(directory: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(directory)
}

/// Runs a call that waits for another holder of the lock. On a multi-thread runtime the
/// worker hands its other tasks over first, so the holder, which may be one of them awaiting
/// the network, keeps being polled and releases the lock.
fn off_worker<T>(wait: impl FnOnce() -> T) -> T {
    match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(wait)
        }
        _ => wait(),
    }
}

fn open_existing(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).open(path)
}

impl SessionStore for CredentialFile {
    fn lock(&self) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(self.lock_exclusive()?))
    }

    fn load(&self) -> io::Result<Option<Session>> {
        Ok(CredentialFile::load(self)
            .map_err(into_io)?
            .and_then(|device| device.session))
    }

    fn save(&self, session: &Session) -> io::Result<()> {
        self.save_locked(session).map_err(into_io)
    }
}

fn into_io(error: CredentialError) -> io::Error {
    io::Error::other(error)
}

#[cfg(test)]
mod tests;

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

mod seed;

/// A credential file is a few hundred bytes; anything larger is not ours.
const MAX_FILE_BYTES: usize = 16 * 1024;
/// Keeps `<name>.toml.lock` below the usual 255-byte file name limit.
const MAX_NAME_BYTES: usize = 200;

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
fn file_stem(address: &HubAddress) -> String {
    format!("{}_{}", address.host(), address.port()).replace(':', "_")
}

/// What the file holds, as stored.
#[derive(Serialize, Deserialize)]
struct Stored {
    address: String,
    seed: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<Session>,
}

/// The device key and tokens read from a credential file.
pub(crate) struct Device {
    pub(crate) key: DeviceKey,
    pub(crate) session: Option<Session>,
}

/// The credential file of one hub. Also the [`SessionStore`] its client rotates tokens in.
#[derive(Debug)]
pub(crate) struct CredentialFile {
    path: PathBuf,
    lock_path: PathBuf,
    address: HubAddress,
}

impl CredentialFile {
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
    pub(crate) fn load_or_create(&self) -> Result<(Device, bool), CredentialError> {
        let _lock = self.lock_exclusive().map_err(CredentialError::Prepare)?;
        if let Some(device) = self.load()? {
            return Ok((device, false));
        }
        let key = DeviceKey::generate()
            .map_err(|error| CredentialError::Prepare(io::Error::other(error)))?;
        self.write(&key, None)?;
        Ok((Device { key, session: None }, true))
    }

    /// Replaces the tokens, keeping the key.
    pub(crate) fn save_session(&self, session: &Session) -> Result<(), CredentialError> {
        self.replace_session(Some(session))
    }

    /// Forgets the tokens but keeps the key, so the device signs in again as itself.
    pub(crate) fn clear_session(&self) -> Result<(), CredentialError> {
        self.replace_session(None)
    }

    fn replace_session(&self, session: Option<&Session>) -> Result<(), CredentialError> {
        let device = self
            .load()?
            .ok_or_else(|| CredentialError::Prepare(io::Error::from(io::ErrorKind::NotFound)))?;
        self.write(&device.key, session)
    }

    fn write(&self, key: &DeviceKey, session: Option<&Session>) -> Result<(), CredentialError> {
        self.prepare_directory().map_err(CredentialError::Prepare)?;
        let stored = Stored {
            address: self.address.to_string(),
            seed: seed::encode(&key.seed()),
            session: session.cloned(),
        };
        let text = toml::to_string(&stored)?;
        write_private_atomic(&self.path, text.as_bytes())?;
        Ok(())
    }

    /// Removes the credentials of the hub; no file is not an error.
    pub(crate) fn erase(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    fn prepare_directory(&self) -> io::Result<()> {
        let Some(parent) = self.path.parent() else {
            return Ok(());
        };
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        builder.create(parent)
    }

    /// Opens the lock file, creating it private, and waits for the exclusive lock.
    fn lock_exclusive(&self) -> io::Result<File> {
        self.prepare_directory()?;
        let file = match create_private(&self.lock_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                open_existing(&self.lock_path)?
            }
            Err(error) => return Err(error.into_io_error()),
        };
        file.lock()?;
        Ok(file)
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
        self.save_session(session).map_err(into_io)
    }
}

fn into_io(error: CredentialError) -> io::Error {
    io::Error::other(error)
}

#[cfg(test)]
mod tests;

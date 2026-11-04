//! The user's `authorized_keys` on a server, edited through the SFTP of a session the user
//! already opened.
//!
//! [`SftpAuthorizedKeys`] reads the whole file and replaces it atomically only if it is still
//! what the change was computed from, so a concurrent edit by the user is reported, never
//! overwritten. [`HostDirectory`] remembers the live sessions of saved connections.
use crate::{SshConfig, host_keys::HostKeys};
use futures::future::BoxFuture;
use nocterm_core::error::SharedCause;
use nocterm_session::{
    FsError, RemoteFs,
    fs::{DownloadReadError, UploadMode, path, read_bounded},
};
use nocterm_ssh_keys::authorized_keys::{
    AgentHosts, AuthorizedKeysFile, ConnectedHost, ConnectedHosts, HostAccess, HostAccessError,
    HostIdentity, HostSession, MAX_AUTHORIZED_KEYS_BYTES,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError, Weak},
};

const CHUNK: usize = 32 * 1024;
const DIRECTORY_PERMISSIONS: u32 = 0o700;
const FILE_PERMISSIONS: u32 = 0o600;

/// `~/.ssh/authorized_keys` of the user of one session.
pub struct SftpAuthorizedKeys {
    fs: Arc<dyn RemoteFs>,
}

impl SftpAuthorizedKeys {
    /// The file reached through `fs`.
    pub fn new(fs: Arc<dyn RemoteFs>) -> Self {
        Self { fs }
    }
}

fn host(error: FsError) -> HostAccessError {
    HostAccessError::Host(SharedCause::new(error))
}

async fn location(fs: &dyn RemoteFs) -> Result<(String, String), HostAccessError> {
    let home = fs.home().await.map_err(host)?;
    let directory = path::join(&home, ".ssh");
    let file = path::join(&directory, "authorized_keys");
    Ok((directory, file))
}

async fn read_text(fs: &dyn RemoteFs, file: &str) -> Result<String, HostAccessError> {
    let mut download = match fs.download(file).await {
        Ok(download) => download,
        Err(FsError::NotFound { .. }) => return Ok(String::new()),
        Err(error) => return Err(host(error)),
    };
    let bytes = read_bounded(download.as_mut(), MAX_AUTHORIZED_KEYS_BYTES, CHUNK)
        .await
        .map_err(|error| match error {
            DownloadReadError::Read(source) => host(source),
            DownloadReadError::TooLarge { .. } => HostAccessError::TooLarge,
            error @ DownloadReadError::InvalidChunk { .. } => {
                HostAccessError::Host(SharedCause::new(error))
            }
        })?;
    download.close().await.map_err(host)?;
    String::from_utf8(bytes).map_err(|_| HostAccessError::NotText)
}

async fn replace_text(
    fs: &dyn RemoteFs,
    previous: &str,
    updated: String,
) -> Result<(), HostAccessError> {
    if updated.len() > MAX_AUTHORIZED_KEYS_BYTES {
        return Err(HostAccessError::TooLarge);
    }
    let (directory, file) = location(fs).await?;
    if read_text(fs, &file).await? != previous {
        return Err(HostAccessError::Changed);
    }
    if fs.stat(&directory).await.map_err(host)?.is_none() {
        fs.create_dir(&directory).await.map_err(host)?;
        let _ = fs.set_permissions(&directory, DIRECTORY_PERMISSIONS).await;
    }
    let mode = if previous.is_empty() {
        UploadMode::Create
    } else {
        UploadMode::Replace
    };
    let mut upload = match fs.upload(&file, mode).await {
        Ok(upload) => upload,
        // The file appeared since it was read as missing.
        Err(FsError::AlreadyExists { .. }) => return Err(HostAccessError::Changed),
        Err(error) => return Err(host(error)),
    };
    for chunk in updated.as_bytes().chunks(CHUNK) {
        if let Err(error) = upload.write(chunk.to_vec()).await {
            let _ = upload.cancel().await;
            return Err(host(error));
        }
    }
    upload.finish().await.map_err(host)?;
    let _ = fs.set_permissions(&file, FILE_PERMISSIONS).await;
    Ok(())
}

impl AuthorizedKeysFile for SftpAuthorizedKeys {
    fn read(&self) -> BoxFuture<'static, Result<String, HostAccessError>> {
        let fs = self.fs.clone();
        Box::pin(async move {
            let (_, file) = location(fs.as_ref()).await?;
            read_text(fs.as_ref(), &file).await
        })
    }

    fn replace(
        &self,
        previous: String,
        updated: String,
    ) -> BoxFuture<'static, Result<(), HostAccessError>> {
        let fs = self.fs.clone();
        Box::pin(async move { replace_text(fs.as_ref(), &previous, updated).await })
    }
}

struct Live {
    identity: HostIdentity,
    fs: Weak<dyn RemoteFs>,
}

/// The saved connections that have a live session, for the trust service to reach.
pub struct HostDirectory {
    known_hosts: Vec<std::path::PathBuf>,
    hosts: Mutex<HashMap<String, Live>>,
}

impl HostDirectory {
    /// A directory that reads recorded host keys from the files `config` names.
    pub fn new(config: &SshConfig) -> Self {
        let mut known_hosts = vec![config.known_hosts.clone()];
        known_hosts.extend(config.read_only_known_hosts.iter().cloned());
        Self {
            known_hosts,
            hosts: Mutex::new(HashMap::new()),
        }
    }

    fn recorded_fingerprint(&self, host: &str, port: u16) -> Option<String> {
        let (writable, read_only) = self.known_hosts.split_first()?;
        HostKeys::load(writable.clone(), read_only.to_vec())
            .ok()?
            .fingerprints(host, port)
            .into_iter()
            .next()
    }
}

impl ConnectedHosts for HostDirectory {
    fn connected(&self, mut host: ConnectedHost) {
        if host.identity.host_key_fingerprint.is_empty() {
            host.identity.host_key_fingerprint = self
                .recorded_fingerprint(&host.identity.host, host.port)
                .unwrap_or_else(|| "unrecorded".to_owned());
        }
        self.hosts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                host.connection,
                Live {
                    identity: host.identity,
                    fs: Arc::downgrade(&host.fs),
                },
            );
    }
}

impl AgentHosts for HostDirectory {
    fn sessions(&self) -> Vec<HostSession> {
        let hosts = self.hosts.lock().unwrap_or_else(PoisonError::into_inner);
        let mut sessions: Vec<_> = hosts
            .iter()
            .filter(|(_, live)| live.fs.strong_count() > 0)
            .map(|(connection, live)| HostSession {
                connection: connection.clone(),
                user: live.identity.user.clone(),
                host: live.identity.host.clone(),
            })
            .collect();
        sessions.sort_by(|a, b| a.connection.cmp(&b.connection));
        sessions
    }

    fn access(&self, connection: &str) -> Result<HostAccess, HostAccessError> {
        let mut hosts = self.hosts.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(live) = hosts.get(connection) else {
            return Err(HostAccessError::NotConnected);
        };
        let Some(fs) = live.fs.upgrade().filter(|fs| fs.capabilities().rename) else {
            hosts.remove(connection);
            return Err(HostAccessError::NotConnected);
        };
        Ok(HostAccess {
            identity: live.identity.clone(),
            file: Arc::new(SftpAuthorizedKeys::new(fs)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_session::testing::InMemoryFs;

    fn file_on(fs: &InMemoryFs) -> SftpAuthorizedKeys {
        SftpAuthorizedKeys::new(Arc::new(fs.clone()))
    }

    async fn path_of(fs: &InMemoryFs) -> String {
        let home = fs.home().await.expect("home");
        path::join(&path::join(&home, ".ssh"), "authorized_keys")
    }

    #[tokio::test]
    async fn a_missing_file_reads_as_empty_and_is_created_by_the_first_edit() {
        let fs = InMemoryFs::default();
        let file = file_on(&fs);
        assert_eq!(file.read().await.expect("read"), "");
        file.replace(String::new(), "ssh-ed25519 AAAA one\n".into())
            .await
            .expect("create");
        assert_eq!(file.read().await.expect("read"), "ssh-ed25519 AAAA one\n");
    }

    #[tokio::test]
    async fn an_edit_replaces_the_file_and_leaves_other_lines_in_place() {
        let fs = InMemoryFs::default();
        let file = file_on(&fs);
        let original = "# mine\nssh-ed25519 AAAA one\n";
        file.replace(String::new(), original.into())
            .await
            .expect("seed");
        let updated = format!("{original}cert-authority ssh-ed25519 BBBB ca\n");
        file.replace(original.into(), updated.clone())
            .await
            .expect("edit");
        let stored = fs.bytes(&path_of(&fs).await).expect("bytes");
        assert_eq!(String::from_utf8(stored).expect("text"), updated);
    }

    #[tokio::test]
    async fn an_edit_computed_from_a_stale_read_is_refused() {
        let fs = InMemoryFs::default();
        let file = file_on(&fs);
        file.replace(String::new(), "one\n".into())
            .await
            .expect("seed");
        let error = file
            .replace("other\n".into(), "two\n".into())
            .await
            .expect_err("stale");
        assert!(matches!(error, HostAccessError::Changed));
        assert_eq!(file.read().await.expect("read"), "one\n");
    }

    #[tokio::test]
    async fn a_binary_file_is_not_edited() {
        let fs = InMemoryFs::default();
        let file = file_on(&fs);
        file.replace(String::new(), "x\n".into())
            .await
            .expect("seed");
        fs.seed_file(path_of(&fs).await.as_str(), vec![0xff, 0xfe])
            .expect("binary");
        assert!(matches!(
            file.read().await.expect_err("binary"),
            HostAccessError::NotText
        ));
    }
}

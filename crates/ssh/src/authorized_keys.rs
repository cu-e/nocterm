//! The user's `authorized_keys` on a server, edited through the SFTP of a session the user
//! already opened.
//!
//! [`SftpAuthorizedKeys`] uses one private no-clobber lock shared by cooperating sessions
//! and devices, then compares the original again before atomic publication. SFTP has no
//! compare-and-swap: uncooperative writers can still race the final comparison. Interrupted
//! edits leave the lock for explicit recovery. [`HostDirectory`] remembers live connections.
mod edit_lock;
use crate::{SshConfig, host_keys::HostKeys};
use edit_lock::EditLock;
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
    edits: Arc<futures::lock::Mutex<()>>,
}

impl SftpAuthorizedKeys {
    /// The file reached through `fs`.
    pub fn new(fs: Arc<dyn RemoteFs>) -> Self {
        Self {
            fs,
            edits: Arc::default(),
        }
    }
}

fn host(error: FsError) -> HostAccessError {
    match error {
        error @ FsError::Unsupported(_) => HostAccessError::Unsupported(SharedCause::new(error)),
        error => HostAccessError::Host(SharedCause::new(error)),
    }
}

async fn location(fs: &dyn RemoteFs) -> Result<(String, String), HostAccessError> {
    let home = fs.home().await.map_err(host)?;
    let directory = path::join(&home, ".ssh");
    let file = path::join(&directory, "authorized_keys");
    Ok((directory, file))
}

async fn read_document(fs: &dyn RemoteFs, file: &str) -> Result<Option<String>, HostAccessError> {
    let mut download = match fs.download(file).await {
        Ok(download) => download,
        Err(FsError::NotFound { .. }) => return Ok(None),
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
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| HostAccessError::NotText)
}

async fn replace_text(
    fs: &dyn RemoteFs,
    file: &str,
    previous: &str,
    updated: String,
) -> Result<(), HostAccessError> {
    let original = read_document(fs, file).await?;
    if original.as_deref().unwrap_or_default() != previous {
        return Err(HostAccessError::Changed);
    }
    let mode = if original.is_none() {
        UploadMode::Create
    } else {
        UploadMode::Replace
    };
    let mut upload = match fs.upload(file, mode).await {
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
    let current = read_document(fs, file).await;
    if !matches!(&current, Ok(document) if *document == original) {
        let _ = upload.cancel().await;
        current?;
        return Err(HostAccessError::Changed);
    }
    // Atomic publication is not a CAS with an uncooperative external writer.
    upload.finish().await.map_err(host)?;
    let _ = fs.set_permissions(file, FILE_PERMISSIONS).await;
    Ok(())
}

impl AuthorizedKeysFile for SftpAuthorizedKeys {
    fn read(&self) -> BoxFuture<'static, Result<String, HostAccessError>> {
        let fs = self.fs.clone();
        Box::pin(async move {
            let (_, file) = location(fs.as_ref()).await?;
            Ok(read_document(fs.as_ref(), &file).await?.unwrap_or_default())
        })
    }

    fn replace(
        &self,
        previous: String,
        updated: String,
    ) -> BoxFuture<'static, Result<(), HostAccessError>> {
        let fs = self.fs.clone();
        let edits = self.edits.clone();
        Box::pin(async move {
            let _edit = edits.lock().await;
            if updated.len() > MAX_AUTHORIZED_KEYS_BYTES {
                return Err(HostAccessError::TooLarge);
            }
            let (directory, file) = location(fs.as_ref()).await?;
            let lock = EditLock::acquire(fs.as_ref(), &directory).await?;
            let result = replace_text(fs.as_ref(), &file, &previous, updated).await;
            if let Err(error) = lock.release(fs.as_ref()).await {
                // An acknowledged data publication must not be reported as failed.
                // Next acquisition will refuse the orphan and show its recovery path.
                tracing::warn!(%error, "could not release authorized_keys edit lock");
            }
            result
        })
    }
}

struct Live {
    identity: HostIdentity,
    fs: Weak<dyn RemoteFs>,
    edits: Arc<futures::lock::Mutex<()>>,
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
    fn prepare(&self, mut host: ConnectedHost) -> ConnectedHost {
        if host.identity.host_key_fingerprint.is_empty() {
            host.identity.host_key_fingerprint = self
                .recorded_fingerprint(&host.identity.host, host.port)
                .unwrap_or_else(|| "unrecorded".to_owned());
        }
        host
    }
    fn connected(&self, host: ConnectedHost) {
        let mut hosts = self.hosts.lock().unwrap_or_else(PoisonError::into_inner);
        let source = Arc::downgrade(&host.fs);
        let edits = hosts
            .get(&host.connection)
            .filter(|live| Weak::ptr_eq(&live.fs, &source))
            .map(|live| live.edits.clone())
            .unwrap_or_default();
        hosts.insert(
            host.connection,
            Live {
                identity: host.identity,
                fs: source,
                edits,
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
            file: Arc::new(SftpAuthorizedKeys {
                fs,
                edits: live.edits.clone(),
            }),
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
    async fn an_existing_empty_file_can_receive_its_first_key() {
        let fs = InMemoryFs::default();
        fs.seed_file(&path_of(&fs).await, Vec::new()).unwrap();
        let file = file_on(&fs);
        file.replace(String::new(), "key\n".into()).await.unwrap();
        assert_eq!(file.read().await.unwrap(), "key\n");
    }

    #[tokio::test]
    async fn a_change_during_upload_is_preserved_and_reported() {
        use futures::FutureExt as _;
        use nocterm_session::testing::{FsOperation, Gate};
        let fs = InMemoryFs::default();
        let path = path_of(&fs).await;
        fs.seed_file(&path, b"original\n".to_vec()).unwrap();
        let gate = Gate::pending();
        fs.suspend_next(FsOperation::Write, &path, gate.clone());
        let file = file_on(&fs);
        let mut edit = file.replace("original\n".into(), "updated\n".into());
        assert!(edit.as_mut().now_or_never().is_none());
        fs.seed_file(&path, b"external\n".to_vec()).unwrap();
        gate.release();
        assert!(matches!(edit.await, Err(HostAccessError::Changed)));
        assert_eq!(file.read().await.unwrap(), "external\n");
    }

    async fn lock_path(fs: &InMemoryFs) -> String {
        path::join(
            path::parent(&path_of(fs).await),
            ".nocterm-authorized_keys.lock",
        )
    }

    #[tokio::test]
    async fn independent_sessions_refuse_a_conflicting_edit_then_retry_after_release() {
        use futures::FutureExt as _;
        use nocterm_session::testing::{FsOperation, Gate};
        let fs = InMemoryFs::default();
        let path = path_of(&fs).await;
        fs.seed_file(&path, b"original\n".to_vec()).unwrap();
        let gate = Gate::pending();
        fs.suspend_next(FsOperation::Write, &path, gate.clone());
        // Distinct captured adapter objects and distinct local mutexes share only
        // the remote tree, as separate SFTP sessions/devices do on one server.
        let first = file_on(&fs);
        let second = file_on(&fs);
        let mut edit = first.replace("original\n".into(), "original\nfirst\n".into());
        assert!(edit.as_mut().now_or_never().is_none());
        assert!(matches!(
            second.replace("original\n".into(), "second\n".into()).await,
            Err(HostAccessError::EditLocked { path }) if path == lock_path(&fs).await
        ));
        assert_eq!(second.read().await.unwrap(), "original\n");
        gate.release();
        edit.await.unwrap();
        assert!(fs.bytes(&lock_path(&fs).await).is_err());
        second
            .replace(
                "original\nfirst\n".into(),
                "original\nfirst\nsecond\n".into(),
            )
            .await
            .unwrap();
        assert_eq!(first.read().await.unwrap(), "original\nfirst\nsecond\n");
        assert!(fs.bytes(&lock_path(&fs).await).is_err());
    }

    #[tokio::test]
    async fn simultaneous_lock_stages_publish_without_clobbering_the_winner() {
        use futures::FutureExt as _;
        use nocterm_session::testing::{FsOperation, Gate};
        let fs = InMemoryFs::default();
        let path = path_of(&fs).await;
        let lock = lock_path(&fs).await;
        fs.seed_file(&path, b"original\n".to_vec()).unwrap();
        let first_finish = Gate::pending();
        fs.suspend_next(FsOperation::Finish, &lock, first_finish.clone());
        let first = file_on(&fs);
        let mut loser = first.replace("original\n".into(), "loser\n".into());
        assert!(loser.as_mut().now_or_never().is_none());
        let second_write = Gate::pending();
        fs.suspend_next(FsOperation::Write, &path, second_write.clone());
        let second = file_on(&fs);
        let mut winner = second.replace("original\n".into(), "winner\n".into());
        assert!(winner.as_mut().now_or_never().is_none());
        let token = fs.bytes(&lock).unwrap();
        first_finish.release();
        assert!(matches!(
            loser.await,
            Err(HostAccessError::EditLocked { .. })
        ));
        assert_eq!(fs.bytes(&lock).unwrap(), token);
        second_write.release();
        winner.await.unwrap();
        assert_eq!(first.read().await.unwrap(), "winner\n");
        assert!(fs.bytes(&lock).is_err());
    }

    #[tokio::test]
    async fn a_cancelled_edit_leaves_a_lock_that_another_session_never_steals() {
        use futures::FutureExt as _;
        use nocterm_session::testing::{FsOperation, Gate};
        let fs = InMemoryFs::default();
        let path = path_of(&fs).await;
        fs.seed_file(&path, b"original\n".to_vec()).unwrap();
        fs.suspend_next(FsOperation::Write, &path, Gate::pending());
        let first = file_on(&fs);
        let mut edit = first.replace("original\n".into(), "cancelled\n".into());
        assert!(edit.as_mut().now_or_never().is_none());
        let lock = lock_path(&fs).await;
        let token = fs.bytes(&lock).unwrap();
        drop(edit);
        let second = file_on(&fs);
        assert!(matches!(
            second.replace("original\n".into(), "retry\n".into()).await,
            Err(HostAccessError::EditLocked { .. })
        ));
        assert_eq!(second.read().await.unwrap(), "original\n");
        assert_eq!(fs.bytes(&lock).unwrap(), token);
    }

    #[tokio::test]
    async fn an_existing_orphan_is_refused_without_replacing_its_token() {
        let fs = InMemoryFs::default();
        let path = path_of(&fs).await;
        let lock = lock_path(&fs).await;
        fs.seed_file(&path, b"original\n".to_vec()).unwrap();
        fs.seed_file(&lock, b"interrupted device".to_vec()).unwrap();
        assert!(matches!(
            file_on(&fs).replace("original\n".into(), "updated\n".into()).await,
            Err(HostAccessError::EditLocked { path }) if path == lock
        ));
        assert_eq!(fs.bytes(&path).unwrap(), b"original\n");
        assert_eq!(fs.bytes(&lock).unwrap(), b"interrupted device");
    }

    #[tokio::test]
    async fn an_unsupported_lock_protocol_refuses_the_edit_before_creating_a_lock() {
        let fs = InMemoryFs::default();
        let path = path_of(&fs).await;
        fs.seed_file(&path, b"original\n".to_vec()).unwrap();
        fs.set_capabilities(nocterm_session::FsCapabilities {
            remove: false,
            ..fs.capabilities()
        });
        assert!(matches!(
            file_on(&fs).replace("original\n".into(), "updated\n".into()).await,
            Err(HostAccessError::Unsupported(cause))
                if matches!(cause.downcast_ref::<FsError>(), Some(FsError::Unsupported(_)))
        ));
        assert_eq!(fs.bytes(&path).unwrap(), b"original\n");
        assert!(fs.bytes(&lock_path(&fs).await).is_err());
    }

    #[tokio::test]
    async fn cleanup_preserves_a_lock_whose_owner_was_changed_manually() {
        use futures::FutureExt as _;
        use nocterm_session::testing::{FsOperation, Gate};
        let fs = InMemoryFs::default();
        let path = path_of(&fs).await;
        let lock = lock_path(&fs).await;
        fs.seed_file(&path, b"original\n".to_vec()).unwrap();
        let gate = Gate::pending();
        fs.suspend_next(FsOperation::Write, &path, gate.clone());
        let file = file_on(&fs);
        let mut edit = file.replace("original\n".into(), "updated\n".into());
        assert!(edit.as_mut().now_or_never().is_none());
        fs.seed_file(&lock, b"another owner".to_vec()).unwrap();
        gate.release();
        edit.await.unwrap();
        assert_eq!(file.read().await.unwrap(), "updated\n");
        assert_eq!(fs.bytes(&lock).unwrap(), b"another owner");
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

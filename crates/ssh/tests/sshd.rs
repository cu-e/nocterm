//! End-to-end tests against a real OpenSSH server.
//!
//! Each test starts a private `sshd` on a free local port, as the current
//! user, with its own host key and authorized keys. Without an `sshd` on the
//! machine the tests pass vacuously, unless `NOCTERM_REQUIRE_SSHD` is set (CI
//! sets it, so they cannot silently stop running there).

use std::{
    fs,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
#[path = "cases/file_operations.rs"]
mod file_operations;

#[path = "cases/download.rs"]
mod download;

#[path = "cases/proxy.rs"]
mod proxy;

#[path = "cases/exec.rs"]
mod exec;

#[path = "cases/host_keys.rs"]
mod host_keys;

use nocterm_session::{
    Auth, CloseReason, ConnectRequest, EntryKind, Event, HostKeyDecision, Prompt, PtySize, Secret,
    SecretRequest, Session, SessionError, Target, Transport,
};
use nocterm_ssh::{SshConfig, SshTransport};
use tempfile::TempDir;
use tokio::time::timeout;

const PASSPHRASE: &str = "correct horse";
const EVENT_TIMEOUT: Duration = Duration::from_secs(20);
// Choosing a free port releases its socket before sshd binds it. Serialize
// that interval so concurrent fixtures cannot accidentally choose the same one.
static SERVER_START: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A throwaway OpenSSH server.
struct Sshd {
    dir: TempDir,
    port: u16,
    process: Child,
}

impl Sshd {
    /// Starts a server, or returns `None` when the machine has none.
    fn start() -> Option<Self> {
        let Some(binary) = find_sshd() else {
            assert!(
                std::env::var_os("NOCTERM_REQUIRE_SSHD").is_none(),
                "NOCTERM_REQUIRE_SSHD is set but no sshd was found"
            );
            return None;
        };
        let _starting = SERVER_START.lock().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        keygen(&root.join("host_key"), "");
        keygen(&root.join("plain/id_ed25519"), "");
        keygen(&root.join("encrypted/id_ed25519"), PASSPHRASE);
        fs::create_dir(root.join("empty")).unwrap();
        let authorized = [
            fs::read_to_string(root.join("plain/id_ed25519.pub")).unwrap(),
            fs::read_to_string(root.join("encrypted/id_ed25519.pub")).unwrap(),
        ]
        .concat();
        fs::write(root.join("authorized_keys"), authorized).unwrap();

        let port = free_port();
        fs::write(
            root.join("sshd_config"),
            format!(
                "Port {port}\n\
                 ListenAddress 127.0.0.1\n\
                 HostKey {root}/host_key\n\
                 PidFile none\n\
                 AuthorizedKeysFile {root}/authorized_keys\n\
                 StrictModes no\n\
                 UsePAM no\n\
                 PubkeyAuthentication yes\n\
                 PasswordAuthentication no\n\
                 KbdInteractiveAuthentication no\n\
                 Subsystem sftp internal-sftp\n\
                 LogLevel ERROR\n",
                root = root.display(),
            ),
        )
        .unwrap();

        let process = Command::new(binary)
            .arg("-D")
            .arg("-e")
            .arg("-f")
            .arg(root.join("sshd_config"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .expect("sshd starts");
        let sshd = Self { dir, port, process };
        sshd.wait_until_listening();
        Some(sshd)
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn wait_until_listening(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while TcpStream::connect(("127.0.0.1", self.port)).is_err() {
            assert!(Instant::now() < deadline, "sshd did not start listening");
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// A transport that trusts nothing yet and offers the keys in `keys`.
    fn transport(&self, keys: &str) -> SshTransport {
        SshTransport::new(SshConfig {
            known_hosts: self.root().join("known_hosts"),
            read_only_known_hosts: Vec::new(),
            identity_dir: Some(self.root().join(keys)),
            // The developer's own agent must not influence the outcome.
            use_agent: false,
        })
        .unwrap()
    }

    fn request(&self) -> ConnectRequest {
        ConnectRequest {
            proxy: Default::default(),
            launch: Default::default(),
            target: Target {
                host: "127.0.0.1".into(),
                port: self.port,
                user: current_user(),
            },
            auth: Auth::Auto,
            term: "xterm-256color".into(),
            size: PtySize::default(),
            connect_timeout: Duration::from_secs(10),
            keepalive_interval: None,
        }
    }

    fn host_label(&self) -> String {
        format!("[127.0.0.1]:{}", self.port)
    }

    fn host_fingerprint(&self) -> String {
        let output = Command::new("ssh-keygen")
            .args(["-l", "-E", "sha256", "-f"])
            .arg(self.root().join("host_key.pub"))
            .output()
            .unwrap();
        String::from_utf8(output.stdout)
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_owned()
    }
}

impl Drop for Sshd {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

fn find_sshd() -> Option<PathBuf> {
    // sshd refuses to start unless invoked by absolute path.
    ["/usr/sbin/sshd", "/usr/bin/sshd", "/usr/local/sbin/sshd"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.exists())
}

fn keygen(path: &Path, passphrase: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let status = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-C", "", "-N", passphrase, "-f"])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn current_user() -> String {
    let output = Command::new("id").arg("-un").output().unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

async fn next_event(session: &Session) -> Event {
    timeout(EVENT_TIMEOUT, session.next_event())
        .await
        .expect("the session went quiet")
        .expect("the session ended without saying why")
}

/// Drives a session until the shell runs, answering prompts with `answer`.
async fn connect(session: &Session, mut answer: impl FnMut(Prompt)) {
    loop {
        match next_event(session).await {
            Event::Connected => return,
            Event::Prompt(prompt) => answer(prompt),
            Event::Closed(reason) => panic!("closed while connecting: {reason:?}"),
            Event::Connecting(_) | Event::Output(_) => {}
        }
    }
}

/// Drives a session to its end, answering prompts with `answer`.
async fn closed(session: &Session, mut answer: impl FnMut(Prompt)) -> CloseReason {
    loop {
        match next_event(session).await {
            Event::Closed(reason) => return reason,
            Event::Prompt(prompt) => answer(prompt),
            _ => {}
        }
    }
}

fn accept(prompt: Prompt) {
    match prompt {
        Prompt::UnknownHostKey { reply, .. } => reply.send(HostKeyDecision::AcceptOnce),
        Prompt::ChangedHostKey { .. } => panic!("unexpected changed host key"),
        Prompt::Secret { request, .. } => panic!("unexpected question: {request:?}"),
    }
}

fn no_prompts(prompt: Prompt) {
    panic!("unexpected prompt: {prompt:?}");
}

/// Collects output until `needle` appears in it.
async fn output_containing(session: &Session, needle: &str) -> String {
    let mut output = Vec::new();
    loop {
        match next_event(session).await {
            Event::Output(bytes) => {
                output.extend(bytes);
                let text = String::from_utf8_lossy(&output);
                if text.contains(needle) {
                    return text.into_owned();
                }
            }
            Event::Closed(reason) => panic!("closed before printing {needle:?}: {reason:?}"),
            _ => {}
        }
    }
}

#[tokio::test]
async fn the_remote_file_system_is_browsable() {
    let Some(sshd) = Sshd::start() else { return };
    let browsed = sshd.root().join("browsed");
    fs::create_dir_all(browsed.join("folder")).unwrap();
    fs::write(browsed.join("file.txt"), "12345").unwrap();
    std::os::unix::fs::symlink(browsed.join("folder"), browsed.join("link")).unwrap();
    std::os::unix::fs::symlink(browsed.join("missing"), browsed.join("dangling")).unwrap();

    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let fs = session.fs().expect("SSH sessions browse files");

    let home = fs.home().await.unwrap();
    assert!(home.starts_with('/'), "home is absolute: {home}");

    let mut entries = fs.read_dir(browsed.to_str().unwrap()).await.unwrap();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    let summary: Vec<_> = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.kind, entry.is_symlink))
        .collect();
    assert_eq!(
        summary,
        [
            ("dangling", EntryKind::Other, true),
            ("file.txt", EntryKind::File, false),
            ("folder", EntryKind::Directory, false),
            ("link", EntryKind::Directory, true),
        ]
    );
    assert_eq!(entries[1].size, Some(5));

    let missing = fs.read_dir("/nonexistent/nocterm").await.unwrap_err();
    assert_eq!(
        missing,
        nocterm_session::FsError::NotFound {
            path: "/nonexistent/nocterm".into()
        }
    );

    session.close();
    closed(&session, no_prompts).await;
    assert_eq!(
        fs.home().await.unwrap_err(),
        nocterm_session::FsError::NotConnected
    );
}

#[tokio::test]
async fn an_encrypted_key_is_unlocked_with_its_passphrase() {
    let Some(sshd) = Sshd::start() else { return };

    let transport = sshd.transport("encrypted");
    let session = transport.open(sshd.request());
    let mut retries = Vec::new();
    connect(&session, |prompt| match prompt {
        Prompt::UnknownHostKey { reply, .. } => reply.send(HostKeyDecision::AcceptOnce),
        Prompt::Secret {
            request: SecretRequest::KeyPassphrase { retry, .. },
            reply,
        } => {
            retries.push(retry);
            let guess = if retry { PASSPHRASE } else { "wrong" };
            reply.send(Some(Secret::new(guess)));
        }
        other => panic!("unexpected prompt: {other:?}"),
    })
    .await;

    assert_eq!(retries, [false, true]);
    session.close();
}

#[tokio::test]
async fn without_a_usable_key_sign_in_fails() {
    let Some(sshd) = Sshd::start() else { return };

    let transport = sshd.transport("empty");
    let session = transport.open(sshd.request());

    assert_eq!(
        closed(&session, accept).await,
        CloseReason::Failed(SessionError::Other(format!(
            "{}@127.0.0.1 requires an SSH key and does not allow password sign-in. Set Authentication to Key file and select a Private key, or load an authorized key into your SSH agent.",
            current_user()
        )))
    );
}

#[tokio::test]
async fn an_unreachable_host_is_reported() {
    let Some(sshd) = Sshd::start() else { return };
    let mut request = sshd.request();
    request.target.port = free_port();

    let transport = sshd.transport("plain");
    let session = transport.open(request);

    let reason = closed(&session, no_prompts).await;
    assert!(
        matches!(
            reason,
            CloseReason::Failed(SessionError::Unreachable { ref host, .. }) if host == "127.0.0.1"
        ),
        "{reason:?}"
    );
}

fn partials(directory: &Path) -> Vec<PathBuf> {
    fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".nocterm-")
                .then_some(path)
        })
        .collect()
}
async fn wait_no_partials(directory: &Path) {
    timeout(Duration::from_secs(5), async {
        while !partials(directory).is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("owned partial file was not cleaned up");
}

#[tokio::test]
async fn staged_upload_keeps_destination_untouched_until_explicit_atomic_publish() {
    use nocterm_session::fs::UploadMode;
    let Some(sshd) = Sshd::start() else { return };
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let remote = session.fs().unwrap();
    let folder = sshd.root().join("uploads");
    fs::create_dir(&folder).unwrap();
    let target = folder.join("new.bin");
    let path = target.to_str().unwrap();
    let mut writer = remote.upload(path, UploadMode::Create).await.unwrap();
    assert!(
        writer.write(vec![0; 32 * 1024 + 1]).await.is_err(),
        "oversized writes must be rejected before enqueueing data"
    );
    let first = vec![0x15; 32 * 1024];
    let second = vec![0xe3; 19_307];
    writer.write(first.clone()).await.unwrap();
    writer.write(second.clone()).await.unwrap();
    assert!(!target.exists());
    assert_eq!(partials(&folder).len(), 1);
    assert_eq!(writer.finish().await.unwrap(), path);
    let mut expected = first;
    expected.extend(second);
    assert_eq!(fs::read(&target).unwrap(), expected);
    assert!(partials(&folder).is_empty());
    assert!(matches!(
        remote.upload(path, UploadMode::Create).await,
        Err(nocterm_session::FsError::AlreadyExists { .. })
    ));
    assert_eq!(fs::read(&target).unwrap(), expected);
    let mut replace = remote.upload(path, UploadMode::Replace).await.unwrap();
    replace.write(b"replacement".to_vec()).await.unwrap();
    assert_eq!(
        fs::read(&target).unwrap(),
        expected,
        "replacement must leave the old final file intact while streaming"
    );
    replace.finish().await.unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"replacement");
    session.close();
    closed(&session, no_prompts).await;
}

#[tokio::test]
async fn cancel_drop_and_session_shutdown_remove_only_owned_partial_files() {
    use nocterm_session::fs::UploadMode;
    let Some(sshd) = Sshd::start() else { return };
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let remote = session.fs().unwrap();
    let folder = sshd.root().join("uploads");
    fs::create_dir(&folder).unwrap();
    let unrelated = folder.join("keep.txt");
    fs::write(&unrelated, b"keep").unwrap();
    let path = folder.join("new.bin").to_string_lossy().into_owned();
    let mut writer = remote.upload(&path, UploadMode::Create).await.unwrap();
    writer.write(b"partial".to_vec()).await.unwrap();
    writer.cancel().await.unwrap();
    wait_no_partials(&folder).await;
    let mut writer = remote.upload(&path, UploadMode::Create).await.unwrap();
    writer.write(b"partial".to_vec()).await.unwrap();
    drop(writer);
    wait_no_partials(&folder).await;
    let mut writer = remote.upload(&path, UploadMode::Create).await.unwrap();
    writer.write(b"partial".to_vec()).await.unwrap();
    assert_eq!(partials(&folder).len(), 1);
    session.close();
    assert_eq!(
        closed(&session, no_prompts).await,
        CloseReason::ClosedByUser
    );
    wait_no_partials(&folder).await;
    assert_eq!(
        writer.write(b"too late".to_vec()).await,
        Err(nocterm_session::FsError::NotConnected)
    );
    assert_eq!(
        remote.stat(&path).await,
        Err(nocterm_session::FsError::NotConnected)
    );
    assert_eq!(fs::read(unrelated).unwrap(), b"keep");
    assert!(!Path::new(&path).exists());
}

#[tokio::test]
async fn upload_rejects_symlink_destinations_and_late_create_collisions_without_clobber() {
    use nocterm_session::{FsError, fs::UploadMode};
    let Some(sshd) = Sshd::start() else { return };
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let remote = session.fs().unwrap();
    let folder = sshd.root().join("uploads");
    fs::create_dir(&folder).unwrap();
    let directory = folder.join("dir");
    remote
        .create_dir(directory.to_str().unwrap())
        .await
        .unwrap();
    remote
        .create_dir(directory.to_str().unwrap())
        .await
        .unwrap();
    let link = folder.join("link");
    std::os::unix::fs::symlink(&directory, &link).unwrap();
    assert!(matches!(
        remote.create_dir(link.to_str().unwrap()).await,
        Err(FsError::AlreadyExists { .. })
    ));
    assert!(matches!(
        remote
            .upload(link.to_str().unwrap(), UploadMode::Replace)
            .await,
        Err(FsError::AlreadyExists { .. })
    ));
    let destination = folder.join("race.txt");
    let mut upload = remote
        .upload(destination.to_str().unwrap(), UploadMode::Create)
        .await
        .unwrap();
    upload.write(b"incoming".to_vec()).await.unwrap();
    fs::write(&destination, b"newly-created-existing").unwrap();
    assert!(upload.finish().await.is_err());
    assert_eq!(fs::read(&destination).unwrap(), b"newly-created-existing");
    wait_no_partials(&folder).await;
    session.close();
    closed(&session, no_prompts).await;
}

#[tokio::test]
async fn concurrent_small_sftp_uploads_publish_exact_bytes() {
    use futures::{StreamExt as _, TryStreamExt as _};
    use nocterm_session::fs::UploadMode;
    let Some(sshd) = Sshd::start() else { return };
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let remote = session.fs().unwrap();
    let folder = sshd.root().join("uploads");
    fs::create_dir(&folder).unwrap();
    futures::stream::iter(0u8..48)
        .map(|n| {
            let remote = remote.clone();
            let destination = folder.join(format!("file-{n}.bin"));
            async move {
                let mut writer = remote
                    .upload(destination.to_str().unwrap(), UploadMode::Create)
                    .await?;
                writer.write(vec![n; 17]).await?;
                writer.write(vec![n + 1; 33]).await?;
                writer.finish().await?;
                Ok::<_, nocterm_session::FsError>(())
            }
        })
        .buffer_unordered(4)
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    for n in 0u8..48 {
        let mut expected = vec![n; 17];
        expected.extend(vec![n + 1; 33]);
        assert_eq!(
            fs::read(folder.join(format!("file-{n}.bin"))).unwrap(),
            expected
        );
    }
    assert!(partials(&folder).is_empty());
    session.close();
    closed(&session, no_prompts).await;
}

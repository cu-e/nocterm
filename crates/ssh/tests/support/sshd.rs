//! One real private OpenSSH fixture, shared by SSH and app conformance tests.
//! Owns the original sshd/config/keys and bounded session prompt handshake.
//! No production adapter, second SSH login implementation or copied mock server.
use nocterm_session::{
    Auth, ConnectRequest, Event, HostKeyDecision, Prompt, PtySize, Session, Target,
};
use nocterm_ssh::{SshConfig, SshTransport};
use std::{
    fs,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tokio::time::timeout;

pub(crate) const PASSPHRASE: &str = "correct horse";
pub(crate) const EVENT_TIMEOUT: Duration = Duration::from_secs(20);
// Choosing a free port releases its socket before sshd binds it. Serialize
// that interval so concurrent fixtures cannot accidentally choose the same one.
static SERVER_START: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A throwaway OpenSSH server.
pub(crate) struct Sshd {
    dir: TempDir,
    port: u16,
    process: Option<Child>,
}

impl Sshd {
    /// Starts a server, or returns `None` when the machine has none.
    pub(crate) fn start() -> Option<Self> {
        Self::start_with("")
    }

    /// Like [`Sshd::start`], with `extra_config` appended to `sshd_config`.
    pub(crate) fn start_with(extra_config: &str) -> Option<Self> {
        let Some(_) = find_sshd() else {
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
                 LogLevel ERROR\n\
                 {extra_config}",
                root = root.display(),
            ),
        )
        .unwrap();

        let mut sshd = Self {
            dir,
            port,
            process: None,
        };
        sshd.restart().expect("sshd starts");
        Some(sshd)
    }

    pub(crate) fn root(&self) -> &Path {
        self.dir.path()
    }

    fn wait_until_listening(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while TcpStream::connect(("127.0.0.1", self.port())).is_err() {
            assert!(Instant::now() < deadline, "sshd did not start listening");
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    /// Lets the plain key of `keys_of` sign in to this server too.
    pub(crate) fn authorize(&self, keys_of: &Sshd) {
        let key = fs::read_to_string(keys_of.root().join("plain/id_ed25519.pub")).unwrap();
        let path = self.root().join("authorized_keys");
        let mut keys = fs::read_to_string(&path).unwrap();
        keys.push_str(&key);
        fs::write(path, keys).unwrap();
    }

    /// Original transport inputs; never reads the developer's agent or keys.
    pub(crate) fn config(&self, keys: &str) -> SshConfig {
        SshConfig {
            known_hosts: self.root().join("known_hosts"),
            read_only_known_hosts: Vec::new(),
            identity_dir: Some(self.root().join(keys)),
            use_agent: false,
        }
    }
    pub(crate) fn transport(&self, keys: &str) -> SshTransport {
        SshTransport::new(self.config(keys)).unwrap()
    }

    pub(crate) fn stop(&mut self) -> std::io::Result<()> {
        if let Some(mut process) = self.process.take() {
            let _ = process.kill();
            process.wait()?;
        }
        Ok(())
    }
    /// Restart from the original fixture configuration after an explicit edit.
    pub(crate) fn restart(&mut self) -> std::io::Result<()> {
        self.stop()?;
        let binary = find_sshd().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "sshd is unavailable")
        })?;
        self.process = Some(
            Command::new(binary)
                .args(["-D", "-e", "-f"])
                .arg(self.root().join("sshd_config"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .spawn()?,
        );
        self.wait_until_listening();
        Ok(())
    }

    pub(crate) fn request(&self) -> ConnectRequest {
        ConnectRequest {
            proxy: Default::default(),
            launch: Default::default(),
            target: Target {
                host: "127.0.0.1".into(),
                port: self.port(),
                user: current_user(),
            },
            auth: Auth::Auto,
            term: "xterm-256color".into(),
            size: PtySize::default(),
            connect_timeout: Duration::from_secs(10),
            keepalive_interval: None,
            jumps: Vec::new(),
        }
    }
}

impl Drop for Sshd {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn find_sshd() -> Option<PathBuf> {
    // sshd refuses to start unless invoked by absolute path.
    ["/usr/sbin/sshd", "/usr/bin/sshd", "/usr/local/sbin/sshd"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.exists())
}

pub(crate) fn keygen(path: &Path, passphrase: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let status = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-C", "", "-N", passphrase, "-f"])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

pub(crate) fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub(crate) fn current_user() -> String {
    let output = Command::new("id").arg("-un").output().unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

pub(crate) async fn next_event(session: &Session) -> Event {
    timeout(EVENT_TIMEOUT, session.next_event())
        .await
        .expect("the session went quiet")
        .expect("the session ended without saying why")
}

/// Drives a session until the shell runs, answering prompts with `answer`.
pub(crate) async fn connect(session: &Session, mut answer: impl FnMut(Prompt)) {
    loop {
        match next_event(session).await {
            Event::Connected => return,
            Event::Prompt(prompt) => answer(prompt),
            Event::Closed(reason) => panic!("closed while connecting: {reason:?}"),
            Event::Connecting(_) | Event::Output(_) => {}
        }
    }
}

pub(crate) fn accept(prompt: Prompt) {
    match prompt {
        Prompt::UnknownHostKey { reply, .. } => reply.send(HostKeyDecision::AcceptOnce),
        Prompt::ChangedHostKey { .. } => panic!("unexpected changed host key"),
        Prompt::Secret { request, .. } => panic!("unexpected question: {request:?}"),
    }
}

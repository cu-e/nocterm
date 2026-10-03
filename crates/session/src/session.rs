//! The conversation between a session's owner and the transport running it.
//!
//! Both ends talk in messages over channels, never through shared state, so a
//! transport may live on another thread, another runtime or, one day, another
//! process.

use std::{
    fmt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures::channel::oneshot;

use crate::{Auth, RemoteFs, Secret, ShellLaunch, Target};

/// Events a transport may queue before it has to wait for the owner.
///
/// The bound is the back-pressure: when the owner falls behind, the transport
/// stops reading from the network and the remote end slows down, instead of
/// output piling up in memory.
const EVENT_BACKLOG: usize = 256;
const COMMAND_BACKLOG: usize = 64;
const MAX_INPUT: usize = 4 * 1024 * 1024;

/// The size of the terminal, in cells and in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

impl Default for PtySize {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// Everything a transport needs to open a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectRequest {
    /// Resolved explicit network route.
    pub proxy: crate::ProxyConfig,
    /// Explicit launch options; defaults retain the server shell.
    pub launch: ShellLaunch,
    pub target: Target,
    pub auth: Auth,
    /// Terminal type announced to the remote host (its `TERM` variable).
    pub term: String,
    pub size: PtySize,
    pub connect_timeout: Duration,
    /// How often to probe an idle connection; `None` never probes.
    pub keepalive_interval: Option<Duration>,
}

/// What the owner tells a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Bytes to deliver to the remote program: typing, pastes, terminal replies.
    Input(Vec<u8>),
    Resize(PtySize),
    /// End the session.
    Close,
}

/// What a session tells its owner.
#[derive(Debug)]
pub enum Event {
    /// Progress while the session is being set up.
    Connecting(ConnectStage),
    /// The shell is running; output follows.
    Connected,
    /// Bytes the remote program printed.
    Output(Vec<u8>),
    /// The session cannot go on without an answer from the user.
    Prompt(Prompt),
    /// The session is over. Nothing follows.
    Closed(CloseReason),
}

/// Where a connection attempt has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectStage {
    /// Reaching the host.
    Connecting,
    /// Proving who the user is.
    Authenticating,
    /// Starting the remote shell.
    StartingShell,
}

/// Why a session ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseReason {
    /// The remote shell exited, with this status when the host reported one.
    Exited(Option<u32>),
    /// The owner asked for it.
    ClosedByUser,
    Failed(SessionError),
}

/// What went wrong with a session, in terms a user can act on.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error("could not reach {host}: {reason}")]
    Unreachable { host: String, reason: String },
    #[error("{host} did not answer within {seconds} seconds")]
    TimedOut { host: String, seconds: u64 },
    #[error("the host key of {host} was not accepted")]
    HostKeyRejected { host: String },
    /// The host presents a different key than the one on record: either the
    /// host was reinstalled or someone is in the middle.
    #[error(
        "the host key of {host} has changed (line {line} of {}); refusing to connect",
        known_hosts.display()
    )]
    HostKeyChanged {
        host: String,
        known_hosts: PathBuf,
        line: usize,
    },
    #[error("{user}@{host} refused every way of signing in that was tried")]
    AuthenticationFailed { user: String, host: String },
    #[error("signing in was cancelled")]
    Cancelled,
    #[error("the connection was lost: {0}")]
    ConnectionLost(String),
    #[error("{0}")]
    Other(String),
}

/// A question only the user can answer.
///
/// Dropping a prompt without replying cancels whatever was waiting on it.
#[derive(Debug)]
pub enum Prompt {
    /// The host presented a key that is not on record.
    UnknownHostKey {
        host: String,
        /// For example `ssh-ed25519`.
        algorithm: String,
        /// The key's SHA-256 fingerprint, as OpenSSH prints it.
        fingerprint: String,
        reply: Reply<HostKeyDecision>,
    },
    /// A secret is needed to sign in.
    Secret {
        request: SecretRequest,
        reply: Reply<Option<Secret>>,
    },
}

/// What to do about a host key that is not on record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKeyDecision {
    Reject,
    /// Connect now; ask again next time.
    AcceptOnce,
    /// Connect and put the key on record.
    AcceptAndRemember,
}

/// Which secret is being asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretRequest {
    /// The account's password; `retry` is set after a wrong one.
    Password { target: Target, retry: bool },
    /// The passphrase protecting a private key file.
    KeyPassphrase { path: PathBuf, retry: bool },
    /// A question the server asks in its own words (one-time codes and such).
    Interactive {
        /// The server's prompt text.
        prompt: String,
        /// Whether the answer may be shown while it is typed.
        echo: bool,
    },
}

/// The way to answer a [`Prompt`].
pub struct Reply<T>(oneshot::Sender<T>);

impl<T> Reply<T> {
    /// A reply and the future that resolves with the answer. The future fails
    /// if the reply is dropped unanswered.
    pub fn channel() -> (Self, oneshot::Receiver<T>) {
        let (sender, receiver) = oneshot::channel();
        (Self(sender), receiver)
    }

    pub fn send(self, answer: T) {
        // The asker may have gone away (the session was closed); that is fine.
        let _ = self.0.send(answer);
    }
}

impl<T> fmt::Debug for Reply<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Reply")
    }
}

/// The owner's end of a session.
///
/// Cloning yields another handle to the same session.
#[derive(Clone)]
pub struct Session {
    commands: async_channel::Sender<Command>,
    pending_bytes: Arc<AtomicUsize>,
    sizes: async_channel::Sender<PtySize>,
    stale_sizes: async_channel::Receiver<PtySize>,
    closing: async_channel::Sender<()>,
    events: async_channel::Receiver<Event>,
    fs: Option<Arc<dyn RemoteFs>>,
}

impl Session {
    /// Delivers bytes to the remote program.
    pub fn input(&self, bytes: impl Into<Vec<u8>>) -> bool {
        let bytes = bytes.into();
        let length = bytes.len();
        if length > MAX_INPUT
            || self.closing.is_closed()
            || self
                .pending_bytes
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                    pending
                        .checked_add(length)
                        .filter(|total| *total <= MAX_INPUT)
                })
                .is_err()
        {
            return false;
        }
        if self.commands.try_send(Command::Input(bytes)).is_err() {
            self.pending_bytes.fetch_sub(length, Ordering::AcqRel);
            return false;
        }
        true
    }

    pub fn resize(&self, size: PtySize) {
        // Only the latest geometry matters. Resizes must not consume input
        // capacity or disappear behind a full input backlog.
        let mut newest = size;
        loop {
            match self.sizes.try_send(newest) {
                Ok(()) | Err(async_channel::TrySendError::Closed(_)) => break,
                Err(async_channel::TrySendError::Full(size)) => {
                    newest = size;
                    let _ = self.stale_sizes.try_recv();
                }
            }
        }
    }

    /// Asks the session to end. [`Event::Closed`] confirms it.
    pub fn close(&self) {
        // A close must interrupt blocked I/O, independently of queued input.
        self.closing.close();
        self.send(Command::Close);
    }

    /// The next event, or `None` once the transport is gone.
    ///
    /// A session has one event stream; give it one reader.
    pub async fn next_event(&self) -> Option<Event> {
        self.events.recv().await.ok()
    }

    /// The remote file system, when the transport has one.
    pub fn fs(&self) -> Option<Arc<dyn RemoteFs>> {
        self.fs.clone()
    }

    fn send(&self, command: Command) {
        // Control commands are best-effort; close also has a separate signal.
        let _ = self.commands.try_send(command);
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Session").finish_non_exhaustive()
    }
}

/// The transport's end of a session.
#[derive(Clone)]
pub struct SessionDriver {
    commands: async_channel::Receiver<Command>,
    pending_bytes: Arc<AtomicUsize>,
    sizes: async_channel::Receiver<PtySize>,
    closing: async_channel::Receiver<()>,
    events: async_channel::Sender<Event>,
}

impl SessionDriver {
    /// Broadcast close signal, independent of command processing/back-pressure.
    /// Also resolves when the last owner is dropped. Multiple observers may wait.
    pub async fn closed(&self) {
        let _ = self.closing.recv().await;
    }
    /// The next command, or `None` once every [`Session`] handle is dropped.
    pub async fn next_command(&self) -> Option<Command> {
        use futures::FutureExt;
        let command = futures::select_biased! {
            size = self.sizes.recv().fuse() => match size {
                Ok(size) => Some(Command::Resize(size)),
                Err(_) => self.commands.recv().await.ok(),
            },
            command = self.commands.recv().fuse() => command.ok(),
        };
        if let Some(Command::Input(bytes)) = &command {
            self.pending_bytes.fetch_sub(bytes.len(), Ordering::AcqRel);
        }
        command
    }

    /// Reports an event. Returns `false` once nobody is listening, which is
    /// the transport's cue to stop.
    pub async fn emit(&self, event: Event) -> bool {
        self.events.send(event).await.is_ok()
    }

    /// Asks the user a question and waits for the answer. `None` means the
    /// prompt was dismissed or the session's owner is gone.
    pub async fn ask<T>(&self, prompt: impl FnOnce(Reply<T>) -> Prompt) -> Option<T> {
        let (reply, answer) = Reply::channel();
        if !self.emit(Event::Prompt(prompt(reply))).await {
            return None;
        }
        answer.await.ok()
    }
}

/// Creates the two ends of a session.
///
/// Transports call this; everything else receives a [`Session`].
pub fn channel(fs: Option<Arc<dyn RemoteFs>>) -> (Session, SessionDriver) {
    let (command_sender, command_receiver) = async_channel::bounded(COMMAND_BACKLOG);
    let pending_bytes = Arc::new(AtomicUsize::new(0));
    let (size_sender, size_receiver) = async_channel::bounded(1);
    let (close_sender, close_receiver) = async_channel::bounded(1);
    let (event_sender, event_receiver) = async_channel::bounded(EVENT_BACKLOG);
    (
        Session {
            commands: command_sender,
            pending_bytes: pending_bytes.clone(),
            sizes: size_sender,
            stale_sizes: size_receiver.clone(),
            closing: close_sender,
            events: event_receiver,
            fs,
        },
        SessionDriver {
            commands: command_receiver,
            pending_bytes,
            sizes: size_receiver,
            closing: close_receiver,
            events: event_sender,
        },
    )
}

/// Something that opens sessions: SSH today; a local shell, a serial line or a
/// session shared by a collaborator are further implementations of this trait.
pub trait Transport: Send + Sync + 'static {
    /// Starts connecting and returns at once. Progress, prompts and failure
    /// all arrive as [`Event`]s on the returned session.
    fn open(&self, request: ConnectRequest) -> Session;
}

#[cfg(test)]
mod tests {
    use futures::{executor::block_on, join};

    use super::*;

    #[test]
    fn commands_and_events_cross_the_channel() {
        let (session, driver) = channel(None);

        session.input("ls\r");
        session.resize(PtySize::default());

        block_on(async {
            assert_eq!(
                driver.next_command().await,
                Some(Command::Resize(PtySize::default()))
            );
            assert_eq!(
                driver.next_command().await,
                Some(Command::Input(b"ls\r".to_vec()))
            );
            session.close();
            assert_eq!(driver.next_command().await, Some(Command::Close));

            assert!(driver.emit(Event::Connected).await);
            assert!(matches!(session.next_event().await, Some(Event::Connected)));
        });
    }

    #[test]
    fn each_end_notices_the_other_going_away() {
        let (session, driver) = channel(None);
        drop(session);
        block_on(async {
            assert_eq!(driver.next_command().await, None);
            assert!(!driver.emit(Event::Connected).await);
        });

        let (session, driver) = channel(None);
        drop(driver);
        assert!(block_on(session.next_event()).is_none());
        session.input("ignored");
    }

    #[test]
    fn pending_input_is_bounded_and_close_wakes_every_observer() {
        let (session, driver) = channel(None);
        assert!(!session.input(vec![0; MAX_INPUT + 1]));
        for _ in 0..COMMAND_BACKLOG {
            assert!(session.input(vec![0; MAX_INPUT / COMMAND_BACKLOG]));
        }
        assert!(!session.input(b"more".to_vec()));
        session.close();
        assert!(!session.input(b"after close".to_vec()));
        block_on(async {
            join!(driver.closed(), driver.closed());
        });
        let (session, driver) = channel(None);
        drop(session);
        block_on(driver.closed());
    }

    #[test]
    fn large_pastes_are_atomic_and_resize_is_coalesced_outside_input_budget() {
        let (session, driver) = channel(None);
        assert!(session.input(vec![1; MAX_INPUT]));
        assert!(!session.input(b"overflow".to_vec()));
        for cols in 1..100 {
            session.resize(PtySize {
                cols,
                ..PtySize::default()
            });
        }
        assert!(matches!(
            block_on(driver.next_command()),
            Some(Command::Resize(PtySize { cols: 99, .. }))
        ));
        assert!(
            matches!(block_on(driver.next_command()), Some(Command::Input(bytes)) if bytes.len() == MAX_INPUT)
        );
        assert!(session.input(b"capacity restored".to_vec()));
    }

    #[test]
    fn asking_resolves_with_the_users_answer() {
        let (session, driver) = channel(None);

        let (answer, ()) = block_on(async {
            join!(
                driver.ask(|reply| Prompt::Secret {
                    request: SecretRequest::Interactive {
                        prompt: "Code:".into(),
                        echo: true,
                    },
                    reply,
                }),
                async {
                    let Some(Event::Prompt(Prompt::Secret { reply, .. })) =
                        session.next_event().await
                    else {
                        panic!("expected a prompt");
                    };
                    reply.send(Some(Secret::new("123456")));
                }
            )
        });

        assert_eq!(answer, Some(Some(Secret::new("123456"))));
    }

    #[test]
    fn dismissing_a_prompt_cancels_the_question() {
        let (session, driver) = channel(None);

        let (answer, ()) = block_on(async {
            join!(
                driver.ask(|reply| Prompt::UnknownHostKey {
                    host: "example.com".into(),
                    algorithm: "ssh-ed25519".into(),
                    fingerprint: "SHA256:abc".into(),
                    reply,
                }),
                async {
                    drop(session.next_event().await);
                }
            )
        });

        assert_eq!(answer, None::<HostKeyDecision>);
    }

    #[test]
    fn secrets_do_not_leak_through_debug() {
        assert_eq!(format!("{:?}", Secret::new("hunter2")), "Secret(…)");
    }
}

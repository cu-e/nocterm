//! The session contract: what the rest of nocterm knows about a remote shell.
//!
//! A [`Session`] is a pair of channels. The owner sends [`Command`]s (input,
//! resize, close) and receives [`Event`]s (output, prompts, the end). How the
//! bytes travel is the business of a [`Transport`], and nothing above this
//! crate depends on one: the UI works the same over SSH, a local shell or a
//! scripted fake in a test.

mod credentials;
pub mod exec;
pub mod fs;
mod launch;
mod options;
mod program;
mod secret;
mod session;
mod settings;
mod target;

pub use credentials::{CredentialFuture, CredentialStore};
pub use exec::{
    Collected, ErrorTail, ExecError, ExecExit, ExecFuture, ExecOutput, ExecRequest, ExecSink,
    HostExec,
};
pub use fs::{DirEntry, EntryKind, FileMetadata, FsCapabilities, FsError, FsFuture, RemoteFs};
pub use secret::Secret;
pub use session::{
    CloseReason, Command, ConnectRequest, ConnectStage, Event, HostKeyDecision, Prompt, PtySize,
    Reply, SecretRequest, Session, SessionDriver, SessionError, Transport, channel,
};
pub use target::{Auth, CredentialId, DEFAULT_PORT, ParseTargetError, Target};

pub use launch::{ShellLaunch, quote_posix, quote_powershell};
pub use program::{ProgramTransport, TerminalRequest};

pub use options::{
    Charset, LoggingOptions, ProxyConfig, SessionOptions, TERM_PRESETS, validate_term,
};
pub use settings::{
    CONNECT_TIMEOUT_RANGE, KEEPALIVE_RANGE, LocalShellSettings, ShellSettings, SshSettings,
};

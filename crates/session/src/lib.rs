//! The session contract: what the rest of nocterm knows about a remote shell.
//! Owns: targets, authentication, launch/override validation, opaque IDs, text errors and execution/FS contracts.
//! Does not own: transport I/O, OS randomness, durable credentials or presentation.
//! Ports provided: Transport, HostExec, RemoteFs, CredentialStore, ProgramOpener,
//! and recording::{OutputLogFactory, OutputLog}.
//! Settings sections: ssh and local (existing adapter configuration). Globals: none.
//! Invariants: IDs use explicit bytes; launch validation performs no I/O or value expansion.
//! InvalidLaunch/InvalidOptions preserve typed causes; errors never retain environment values.
//!
//! A [`Session`] is a pair of channels. The owner sends [`Command`]s (input,
//! resize, close) and receives [`Event`]s (output, prompts, the end). How the
//! bytes travel is the business of a [`Transport`], and nothing above this
//! crate depends on one: the UI works the same over SSH, a local shell or a
//! scripted fake in a test.
//!
//! Owns: session, execution, credential and filesystem contracts; optional in-memory
//! test doubles behind `test-support`.
//! Does not own: real transport, filesystem or credential-storage I/O, or views.
//! Ports used: none. Ports provided: `Transport`, `HostExec`, `RemoteFs`, `CredentialStore`,
//! `ProgramOpener`, `local_fs::{LocalFs, LocalMutations, LocalDirectory, LocalSource, LocalStage}`.
//! Settings sections: SSH and local shell options (adapter-ownership migration debt).
//! Globals: none. Invariants: bounded channels carry ordered output and explicit closure.

#[cfg(feature = "test-support")]
pub mod testing;

mod credentials;
pub mod exec;
pub mod fs;
mod launch;
pub mod local_fs;
mod options;
mod program;
mod program_opener;
pub mod recording;
mod secret;
mod session;
mod settings;
mod target;
mod text;

pub use credentials::{CredentialFuture, CredentialStore, CredentialStoreError};
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
pub use target::{
    Auth, CredentialId, CredentialIdParseError, DEFAULT_PORT, ParseTargetError, Target,
};
pub use text::TextEncodingError;

pub use launch::{LaunchValidationError, ShellLaunch, quote_posix, quote_powershell};
pub use program::{ProgramTransport, TerminalRequest};
pub use program_opener::{ProgramOpenError, ProgramOpenFuture, ProgramOpenRequest, ProgramOpener};

pub use options::{
    Charset, LoggingError, LoggingOptions, ProxyConfig, ProxyError, SessionOptions,
    SessionOptionsError, TERM_PRESETS, TermError, validate_term,
};
pub use settings::{
    CONNECT_TIMEOUT_RANGE, KEEPALIVE_RANGE, LocalShellSettings, ShellSettings, SshSettings,
};

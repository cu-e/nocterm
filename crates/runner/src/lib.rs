//! Independent persistent Cloud control-plane executable.
//!
//! Owns: explicit private R initialization/enrollment/restoration, configured
//! native collaborators, bounded fresh-head service, explicit execution
//! composition (command inbox, coordinator, providers, session broker, outbox
//! and chat summary delivery) when configured, and graceful task drain. Chat summaries
//! name the managed agent from the boot document, or the own runner (ADR 0135).
//! Managed mode (`managed --config <path>`) is started by a pool host. Boot v1 on stdin
//! (ADR 0121) validates and stays control-plane only. With `--control stdin` (ADR 0123)
//! stdin is a framed channel: a boot v2 document, then contract chat access frames that
//! give each chat a memory-only broker token; closing it ends the lease. Boot v3 adds the
//! host's `ssh_egress`. Chat SSH bindings (ADR 0137) are admitted for the configured SSH
//! targets and a managed boot's targets with the egress of `[ssh]` or boot v3; a configured
//! SSH target without device consent signs in with the chat's in-memory key and certificate,
//! and the delivery sweep renews certificates.
//! Does not own: GPUI, model/tool engines, device/account root keys, provider
//! credentials, protocol codecs or server implementation. Execution stays disabled
//! unless both `[execution]` and `[session_targets]` are configured.
//! Control channel reader: a blocking thread owning stdin; the token store, wake hook and
//! brokered providers live in `managed/`.
//! Ports used: hub-io runner identity/session/head responder/chat directory, cloud-runner native
//! journal/current heads and cloud-model finite budgets. Agent file transfers run on
//! one remote-only `nocterm-transfer-runtime` runtime owned by the session broker: it
//! relays between hosts, refuses local upload/download, cancels a chat's transfers when
//! it closes and is frozen and drained with the broker. Provided port: CLI.
//! Settings: a bounded private runner TOML file. Globals: none.
//! show-public exports an explicit bounded setup allowlist, never private configuration.
//! Invariants: independent process lifetime; no implicit reset, secret argv/env,
//! fake execution collaborators or claims of complete tool/account containment.
mod bootstrap;
mod cli;
mod config;
mod control;
mod error;
mod execution;
mod managed;
mod output;
mod public_setup;
mod sessions;
pub use error::RunnerError;
use std::io::Write;

/// CLI entrypoint. Prints the typed error and its cause chain on one line
/// (`RunnerError::chain`); every frame is a static message or an OS error.
pub fn entry() -> std::process::ExitCode {
    match cli::execute() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            // A closed stderr leaves nowhere else to report; the exit code remains.
            let _ = writeln!(std::io::stderr(), "{}", error.chain());
            std::process::ExitCode::FAILURE
        }
    }
}

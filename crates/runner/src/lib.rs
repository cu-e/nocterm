//! Independent persistent Cloud control-plane executable.
//!
//! Owns: explicit private R initialization/enrollment/restoration, configured
//! native collaborators, bounded fresh-head service, explicit execution
//! composition (command inbox, coordinator, providers, session broker, outbox
//! delivery) when configured, and graceful task drain.
//! Managed mode (`managed --config <path>`) is started by a pool host. Boot v1 on stdin
//! (ADR 0121) validates and stays control-plane only. With `--control stdin` (ADR 0123)
//! stdin is a framed channel: a boot v2 document, then contract chat access frames that
//! give each chat a memory-only broker token; closing it ends the lease.
//! Does not own: GPUI, model/tool engines, device/account root keys, provider
//! credentials, protocol codecs or server implementation. Execution stays disabled
//! unless both `[execution]` and `[session_targets]` are configured.
//! Control channel reader: a blocking thread owning stdin; the token store, wake hook and
//! brokered providers live in `managed/`.
//! Ports used: hub-io runner identity/session/head responder, cloud-runner native
//! journal/current heads and cloud-model finite budgets. Provided port: CLI.
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

/// CLI entrypoint. Only static typed top-level diagnostics are printed; native
/// source chains remain available internally and are never expanded into logs.
pub fn entry() -> std::process::ExitCode {
    match cli::execute() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            // A closed stderr leaves nowhere else to report; the exit code remains.
            let _ = writeln!(std::io::stderr(), "{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

//! Independent persistent Cloud control-plane executable.
//!
//! Owns: explicit private R initialization/enrollment/restoration, configured
//! native collaborators, bounded fresh-head service, explicit execution
//! composition (command inbox, coordinator, providers, session broker, outbox
//! delivery) when configured, and graceful task drain.
//! Managed mode (`managed --config <path>`, boot document on stdin) is started by a pool
//! host and is validated here; it stays control-plane only until containment and the
//! broker credential source are composed.
//! Does not own: GPUI, model/tool engines, device/account root keys, provider
//! credentials, protocol codecs or server implementation. Execution stays disabled
//! unless both `[execution]` and `[session_targets]` are configured.
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

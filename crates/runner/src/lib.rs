//! Independent persistent Cloud control-plane executable.
//!
//! Owns: explicit private R initialization/enrollment/restoration, configured
//! native collaborators, bounded fresh-head service and graceful task drain.
//! Does not own: GPUI, model/tool execution, device/account root keys, provider
//! credentials, protocol codecs or server implementation. Execution is disabled.
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
mod public_setup;
mod sessions;
pub use error::RunnerError;

/// CLI entrypoint. Only static typed top-level diagnostics are printed; native
/// source chains remain available internally and are never expanded into logs.
pub fn entry() -> std::process::ExitCode {
    match cli::execute() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

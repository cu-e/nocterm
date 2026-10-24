//! Reading, hashing, storing and removing user plugin packages.
//!
//! The adapter behind the `PluginPackages` port of `nocterm-plugin-api`. A package is one TOML
//! file of at most 256 KiB. It is read without following a link, hashed over the exact buffer
//! that is then parsed, and its manifest is validated before anything is shown for consent.
//!
//! Owns: bounded no-follow reads (through `nocterm_core::fs`), UTF-8 and TOML parsing into the
//! api DTOs, the API version gate, SHA-256 of the parsed bytes, private atomic publication under
//! one package directory, re-hashing at load, and removal.
//! Does not own: manifest rules, grants, consent, enablement, the data of a plugin or running a
//! package (all in the api crate or the host).
//! Ports used/provided: provides `nocterm_plugin_api::PluginPackages`; uses none.
//! Settings section: none. Globals: none.
//! Invariants: the hash of a package is the SHA-256 of the exact bytes that were parsed; a stored
//! package is trusted only after it hashes to the value the user consented to; files are named
//! only by validated `user-` plugin ids, so a package cannot choose its path.

mod parse;
mod store;

pub use parse::{parse_package, sha256};
pub use store::PackageStore;

#[cfg(test)]
mod tests;

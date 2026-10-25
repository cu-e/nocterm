//! Reading, hashing, storing and removing user plugin packages.
//!
//! The adapter behind the `PluginPackages` port of `nocterm-plugin-api`. A package is one file:
//! a TOML manifest of at most 256 KiB (`<id>.toml`, declarative) or a WebAssembly module of at
//! most 2 MiB whose custom section holds that manifest (`<id>.wasm`, code). The kind comes from
//! the file's first bytes. A package is read without following a link, hashed over the exact
//! buffer that is then parsed, and its manifest is validated before anything is shown for
//! consent; a module is not validated or run here.
//!
//! Owns: bounded no-follow reads (through `nocterm_core::fs`), UTF-8 and TOML parsing into the
//! api DTOs, the manifest section of a module, the check that the declared runtime matches the
//! kind of file, the API version gate, SHA-256 of the parsed bytes, private atomic publication
//! under one package directory (one file per plugin, whichever kind), re-hashing at load,
//! removal, reading the packages of a directory for an author's check, and `SessionStore`,
//! which keeps published packages in memory for a developer session.
//! Does not own: manifest rules, grants, consent, enablement, the data of a plugin or running a
//! package (all in the api crate or the host).
//! Ports used/provided: provides `nocterm_plugin_api::PluginPackages`; uses none.
//! Settings section: none. Globals: none.
//! Invariants: the hash of a package is the SHA-256 of the exact bytes that were parsed; a stored
//! package is trusted only after it hashes to the value the user consented to; files are named
//! only by validated `user-` plugin ids, so a package cannot choose its path; a `SessionStore`
//! never writes a file.

mod directory;
mod parse;
mod session;
mod store;

pub use directory::{CheckedFile, DirectoryError, MAX_CHECKED_FILES, read_directory};
pub use parse::{parse_package, sha256};
pub use session::SessionStore;
pub use store::{PackageStore, read_package};

#[cfg(test)]
mod tests;

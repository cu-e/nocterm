//! Private bounded project catalog acquisition/publication and native metadata.
//!
//! Owns: explicit-path catalog reads, core private atomic publication, native
//! identity/time facts and concrete I/O diagnostics retained by domain errors.
//! Does not own: project rules, sessions, views or shutdown.
//! Ports provided: ProjectRepository, ProjectMetadataSource.
//! Ports used: core write_private_atomic. Settings section: none. Globals: none.
//! Invariants: invalid catalogs never become empty; reads are byte bounded;
//! publication replaces the exact final path and preserves old data on failure.
//! Ancestors must be trusted; this is not a held-directory traversal capability.

mod metadata;
mod repository;

pub use metadata::NativeProjectMetadata;
pub use repository::FileProjectRepository;

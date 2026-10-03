//! Shared kernel: standard file locations and atomic, comment-preserving TOML
//! persistence.
//!
//! This crate is the bottom of the dependency graph. It knows nothing about
//! SSH, terminals or the UI; it holds only the primitives that more than one
//! crate needs.

pub mod paths;
pub mod persist;

pub use paths::{Paths, PathsError};
pub use persist::PersistError;

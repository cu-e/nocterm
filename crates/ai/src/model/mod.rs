//! Owned conversation facts. Native capabilities and transport schemas stay outside.
//!
//! `content` and `tools` records are stored in saved transcripts; their JSON shape is
//! a compatibility contract pinned by the version-one history fixtures.
pub mod auth;
pub mod commands;
pub mod content;
pub mod controls;
pub mod diagnostic;
pub mod extensions;
pub mod ids;
pub mod plan;
pub mod session;
pub mod tools;
pub mod usage;

#[cfg(test)]
mod fixture_tests;

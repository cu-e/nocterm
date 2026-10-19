//! Native legacy replay validation. Keys are checked on current replay bytes.
//! This module does not authorize materialization or replace the legacy loader.
use super::json_fields as keys;
use super::json_frames as framing;
mod outputs;
mod plan;
mod scopes;
mod strings;

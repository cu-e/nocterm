//! Public transfer-service behavior over shared filesystem port fakes.
mod support;
use support::*;
#[path = "cases/downloads.rs"]
mod downloads;
#[path = "cases/uploads.rs"]
mod uploads;

//! One top-level table of `settings.toml`, owned by the code that uses it.

use std::fmt::Debug;

use schemars::JsonSchema;
use serde::{Serialize, de::DeserializeOwned};

/// A top-level table of `settings.toml` (`[terminal]`, `[monitor]`, …).
///
/// The crate that uses a section defines it; this crate only reads and
/// writes the tables. Each section is read on its own, so a mistake in one
/// leaves the others working.
pub trait SettingsSection:
    Serialize
    + DeserializeOwned
    + JsonSchema
    + Default
    + Clone
    + PartialEq
    + Debug
    + Send
    + Sync
    + 'static
{
    /// The table's name in `settings.toml`.
    const KEY: &'static str;

    /// Pulls out-of-range values back into range. Runs after every read and
    /// every change, so the application only ever sees values it can act on.
    fn sanitize(&mut self) {}
}

/// Clamps a float into `range`; a NaN is dropped rather than propagated.
pub fn clamp_f32(value: f32, range: std::ops::RangeInclusive<f32>) -> Option<f32> {
    (!value.is_nan()).then(|| value.clamp(*range.start(), *range.end()))
}

/// Trims text; an empty value counts as unset.
pub fn trim_unset(value: &mut Option<String>) {
    *value = value
        .take()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());
}

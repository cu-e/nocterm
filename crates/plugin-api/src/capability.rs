//! The capability table: the only things a user plugin can see or ask for.
//!
//! [`CAPABILITIES`] is the one source. Each entry has a hand-written sealed marker in the module
//! of its domain, naming the data it carries. Released entries never change: a successor entry
//! plus `deprecated` replaces one.

pub mod clock;
mod fields;
pub mod monitor;
pub mod notifications;
pub mod panels;
pub mod storage;
mod table;
pub mod transfers;
pub mod workspace;

pub use fields::{Fields, Value, ValueType};
pub use table::{
    CAPABILITIES, CapabilityEntry, Deprecation, EffectTarget, Family, TableError, capability,
    validate_table,
};

/// The fields of the data a capability observes, or `None` for an id without named fields.
pub fn fields_of(capability: &str) -> Option<&'static [(&'static str, ValueType)]> {
    match capability {
        clock::Now::ID => Some(clock::ClockNow::FIELDS),
        workspace::Kind::ID => Some(workspace::WorkspaceState::FIELDS),
        monitor::UsageNow::ID => Some(monitor::Usage::FIELDS),
        transfers::CountsNow::ID => Some(transfers::Counts::FIELDS),
        _ => None,
    }
}

mod sealed {
    pub trait Sealed {}
}

/// A capability marker. Only this crate implements it.
pub trait Capability: sealed::Sealed {
    /// The id in [`CAPABILITIES`].
    const ID: &'static str;
    /// What the capability observes, or the request an effect carries.
    type Value;
}

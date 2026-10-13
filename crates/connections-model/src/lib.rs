//! Pure saved-connection formats, drafts, transitions and native capability ports.
//!
//! Owns: profile/recent/facts formats, checked mutations, live-profile authority stamps, recent-use intents, drafts, checked OS identities/release policy and observation values.
//! Does not own: DNS, HTTP, files, clocks, GPUI or observation lifecycle.
//! Ports provided: ConnectionRepository, ProfileIds, ConnectionClock and country/flag/RemoteOsDetector observation ports.
//! Ports used: session value types. Settings section: none. Globals: none.
//! Invariants: only checked public addresses reach country lookup; country keys
//! are two lower-case ASCII letters. IDs/timestamps/defaults are supplied values;
//! observations are not connection authority and mutable collection state stays private.

pub mod draft;
pub mod facts;
pub mod observation;
pub mod os;
pub mod os_observation;
pub mod profile_authority;
pub mod profiles;
pub mod recents;
pub mod repository;
pub mod value_ports;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

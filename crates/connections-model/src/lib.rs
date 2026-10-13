//! Pure values and contracts for observing saved connections.
//!
//! Owns: checked country codes, public-address policy, pure profile drafts and validation.
//! Does not own: DNS, HTTP, files, clocks, GPUI or observation lifecycle.
//! Ports provided: PublicAddressResolver, CountryLocator, FlagRepository.
//! Ports used: session value types. Settings section: none. Globals: none.
//! Invariants: only checked public addresses reach country lookup; country keys
//! are two lower-case ASCII letters. Observations are not connection authority.

pub mod draft;
pub mod observation;

#[cfg(feature = "test-support")]
pub mod test_support;

//! Portable encrypted credential rules and explicit lifecycle contracts.
//! Owns: version1 codec, cryptography, binding validation and prepared transitions.
//! Does not own: filesystem, entropy, workers, native authentication or UI.
//! Ports provided: VaultAccess and DeviceUnlockProvider. Ports used: none.
//! Settings: none. Globals: none. Invariants: secrets zeroize; candidates are
//! installed only after the caller commits; encryption randomness is supplied.

//! Settings schema: owned here, including defaults and sanitization; controls stay in the feature.
//! Settings: vault defaults and sanitization; views and settings publication stay outside the domain.
mod access;
mod device_unlock;
mod error;
mod material;
mod model;
mod registration;
mod state;
pub use access::*;
pub use device_unlock::*;
pub use error::VaultError;
pub use material::{CreateMaterial, RotationMaterial, SealNonces};
pub use model::{CredentialBinding, CredentialInfo};
pub use registration::DeviceRegistration;
pub use state::{DeviceEnrollment, EnvelopeInfo, PreparedUnlock, PreparedWrite, VaultState};

#[cfg(feature = "test-support")]
pub mod testing;

mod settings;
pub use settings::*;

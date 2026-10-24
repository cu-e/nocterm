//! From bytes to a checked [`Package`]: no file access.

use nocterm_plugin_api::{
    API_VERSION, HostContext, Manifest, ManifestDto, ManifestHeader, Package, PackageError,
    sanitize_display,
};
use sha2::{Digest as _, Sha256};

use nocterm_plugin_api::ContentHash;

/// Longest parse message kept, in characters.
const MAX_MESSAGE: usize = 160;

/// The SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> ContentHash {
    ContentHash::new(Sha256::digest(bytes).into())
}

/// Hashes `bytes` and checks them as a package.
///
/// The hash is of the exact buffer that is parsed. A package for a newer API is refused before
/// its other keys are read, and unknown keys are refused within the declared API.
pub fn parse_package(bytes: Vec<u8>, reserved_names: &[&str]) -> Result<Package, PackageError> {
    let hash = sha256(&bytes);
    let text = std::str::from_utf8(&bytes).map_err(|_| PackageError::Encoding)?;
    let header: ManifestHeader = toml::from_str(text).map_err(syntax)?;
    header.check(API_VERSION)?;
    let dto: ManifestDto = toml::from_str(text).map_err(syntax)?;
    let host = HostContext::new(API_VERSION, reserved_names);
    let manifest = Manifest::validate(&dto, &host)?;
    Ok(Package::new(hash, manifest, bytes))
}

/// The message of a TOML error without the quoted source line, which the author controls.
fn syntax(error: toml::de::Error) -> PackageError {
    PackageError::Syntax {
        reason: sanitize_display(error.message(), MAX_MESSAGE),
    }
}

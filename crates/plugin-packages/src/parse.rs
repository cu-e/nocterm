//! From bytes to a checked [`Package`]: no file access.

use nocterm_plugin_api::{
    API_VERSION, HostContext, MAX_PACKAGE_BYTES, Manifest, ManifestDto, ManifestHeader, Package,
    PackageError, PackageKind, Runtime, kind, manifest_section, sanitize_display,
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
/// The hash is of the exact buffer that is parsed, the whole file for either kind. A code
/// package (a WebAssembly module) carries its manifest in a custom section; nothing else of the
/// module is read, so a module is validated only when it is instantiated after consent. A
/// package for a newer API is refused before its other keys are read, unknown keys are refused
/// within the declared API, and the declared runtime must be the one the kind of file runs on.
pub fn parse_package(bytes: Vec<u8>, reserved_names: &[&str]) -> Result<Package, PackageError> {
    let hash = sha256(&bytes);
    let package_kind = kind(&bytes);
    let source = match package_kind {
        PackageKind::Manifest => bytes.as_slice(),
        PackageKind::Module => manifest_section(&bytes)?,
    };
    if source.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(PackageError::TooLarge);
    }
    let text = std::str::from_utf8(source).map_err(|_| PackageError::Encoding)?;
    let header: ManifestHeader = toml::from_str(text).map_err(syntax)?;
    header.check(API_VERSION)?;
    let dto: ManifestDto = toml::from_str(text).map_err(syntax)?;
    let host = HostContext::new(API_VERSION, reserved_names);
    let manifest = Manifest::validate(&dto, &host)?;
    let expected = match package_kind {
        PackageKind::Manifest => Runtime::Declarative,
        PackageKind::Module => Runtime::Wasm,
    };
    if manifest.runtime() != expected {
        return Err(PackageError::RuntimeMismatch {
            declared: manifest.runtime(),
            kind: package_kind,
        });
    }
    Ok(Package::new(hash, manifest, bytes))
}

/// The message of a TOML error without the quoted source line, which the author controls.
fn syntax(error: toml::de::Error) -> PackageError {
    PackageError::Syntax {
        reason: sanitize_display(error.message(), MAX_MESSAGE),
    }
}

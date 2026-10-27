//! The two native effects the vault key port needs: operating system
//! entropy for a new key and a bounded read of a key file.

use std::path::Path;

use nocterm_session::Secret;
use nocterm_ssh_keys::{KeyImportError, KeyMaterial};
use zeroize::Zeroizing;

/// The largest private key file read, as when signing in with one.
const MAX_PRIVATE_KEY: usize = 1024 * 1024;

/// Why a new key could not be made.
#[derive(Debug, thiserror::Error)]
pub enum GenerateKeyError {
    #[error("The operating system gave no randomness.")]
    Entropy(#[source] getrandom::Error),
    #[error(transparent)]
    Key(#[from] KeyImportError),
}

/// Why a key file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ReadKeyError {
    #[error("The key file cannot be read.")]
    Io(#[source] std::io::Error),
    #[error("The key file is not text.")]
    NotText,
}

/// A new Ed25519 key from operating system entropy, commented `comment`.
pub fn generate_ed25519(comment: &str) -> Result<KeyMaterial, GenerateKeyError> {
    let mut seed = Zeroizing::new([0_u8; 32]);
    getrandom::fill(seed.as_mut()).map_err(GenerateKeyError::Entropy)?;
    Ok(KeyMaterial::ed25519_from_seed(*seed, comment)?)
}

/// The text of the private key file at `path`, read within bounds from a
/// regular file.
pub fn read_private_key(path: &Path) -> Result<Secret, ReadKeyError> {
    let bytes = crate::file::read(path, MAX_PRIVATE_KEY).map_err(ReadKeyError::Io)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| ReadKeyError::NotText)?;
    Ok(Secret::new(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_generated_keys_differ_and_name_their_comment_nowhere_in_the_secret_debug() {
        let first = generate_ed25519("laptop").unwrap();
        let second = generate_ed25519("laptop").unwrap();
        assert_ne!(first.fingerprint(), second.fingerprint());
        assert!(first.fingerprint().starts_with("SHA256:"));
        let shown = format!("{first:?}");
        assert!(!shown.contains("PRIVATE"), "{shown}");
    }

    #[test]
    fn a_key_file_is_read_as_text_and_refused_beyond_its_bounds() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("key");
        std::fs::write(&path, "key text").unwrap();
        assert_eq!(read_private_key(&path).unwrap().expose(), "key text");
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(matches!(
            read_private_key(&path),
            Err(ReadKeyError::NotText)
        ));
        std::fs::write(&path, vec![b'x'; MAX_PRIVATE_KEY + 1]).unwrap();
        assert!(matches!(read_private_key(&path), Err(ReadKeyError::Io(_))));
        assert!(matches!(
            read_private_key(directory.path()),
            Err(ReadKeyError::Io(_))
        ));
    }
}

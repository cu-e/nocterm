//! Private key material: import, normalization and generation from a seed.
use crate::{KeyImportError, PublicKeyText};
use nocterm_session::Secret;
use ssh_key::{
    Algorithm, LineEnding, PrivateKey,
    private::{Ed25519Keypair, KeypairData},
};
use std::fmt;
use zeroize::Zeroizing;

/// The smallest RSA modulus accepted, in bits.
pub const MIN_RSA_BITS: u32 = 2048;

/// A usable private key, held as its normalized OpenSSH text.
///
/// The key never formats itself: `Debug` shows the fingerprint only.
pub struct KeyMaterial {
    fingerprint: String,
    public: PublicKeyText,
    normalized: Secret,
}

impl KeyMaterial {
    /// Reads an OpenSSH private key, removing its passphrase, and checks that
    /// nocterm accepts the algorithm.
    pub fn import(text: &Secret, passphrase: Option<&Secret>) -> Result<Self, KeyImportError> {
        let parsed =
            PrivateKey::from_openssh(text.expose().trim()).map_err(KeyImportError::Malformed)?;
        let key = if parsed.is_encrypted() {
            let passphrase = passphrase.ok_or(KeyImportError::PassphraseRequired)?;
            parsed
                .decrypt(passphrase.expose())
                .map_err(|error| match error {
                    ssh_key::Error::Crypto => KeyImportError::WrongPassphrase,
                    other => KeyImportError::Malformed(other),
                })?
        } else {
            parsed
        };
        Self::accept(key)
    }

    /// An Ed25519 key derived from `seed`. The caller supplies the entropy.
    pub fn ed25519_from_seed(seed: [u8; 32], comment: &str) -> Result<Self, KeyImportError> {
        let seed = Zeroizing::new(seed);
        let pair = Ed25519Keypair::from_seed(&seed);
        let key = PrivateKey::new(KeypairData::Ed25519(pair), comment)
            .map_err(KeyImportError::Malformed)?;
        Self::accept(key)
    }

    fn accept(key: PrivateKey) -> Result<Self, KeyImportError> {
        match key.algorithm() {
            Algorithm::Ed25519 | Algorithm::Ecdsa { .. } => {}
            Algorithm::Rsa { .. } => {
                let bits = key
                    .public_key()
                    .key_data()
                    .rsa()
                    .map_or(0, ssh_key::public::RsaPublicKey::key_size);
                if bits < MIN_RSA_BITS {
                    return Err(KeyImportError::TooWeak { bits });
                }
            }
            other => {
                return Err(KeyImportError::Unsupported {
                    algorithm: other.to_string(),
                });
            }
        }
        let text = key
            .to_openssh(LineEnding::LF)
            .map_err(KeyImportError::Malformed)?;
        let public = PublicKeyText::of(key.public_key())?;
        Ok(Self {
            fingerprint: public.fingerprint().to_owned(),
            public,
            normalized: Secret::new(text.as_str()),
        })
    }

    /// The public half, as the text hosts and the vault compare.
    pub fn public_key(&self) -> PublicKeyText {
        self.public.clone()
    }

    /// The `SHA256:…` fingerprint `ssh-keygen -lf` prints.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// OpenSSH text without encryption and with LF line endings: the form kept in
    /// the vault, where its encryption is the key's protection.
    pub fn to_secret(&self) -> Secret {
        self.normalized.clone()
    }
}

impl fmt::Debug for KeyMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "KeyMaterial({})", self.fingerprint())
    }
}

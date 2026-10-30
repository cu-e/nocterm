//! The personal user CA that vouches for the ephemeral keys of agents.
//!
//! A [`UserCa`] is an Ed25519 key. It certifies a runner's ephemeral public key as an OpenSSH
//! user certificate that is narrower than any login the user has: only the agent principal,
//! at most 24 hours, only from the named source addresses, and none of the OpenSSH
//! extensions except a terminal. Not even port, agent or X11 forwarding is granted, because an
//! OpenSSH certificate grants nothing it does not list.
use crate::{KeyImportError, PublicKeyText};
use ssh_key::{
    Certificate, HashAlg, PrivateKey,
    certificate::{Builder, CertType},
    private::{Ed25519Keypair, KeypairData},
    public::{Ed25519PublicKey, KeyData},
};
use zeroize::Zeroizing;

/// Every principal of an agent starts with this text.
pub const PRINCIPAL_PREFIX: &str = "nocterm-agent-";
/// The longest life of one certificate: 24 hours.
pub const MAX_VALIDITY_MS: i64 = 24 * 60 * 60 * 1000;
/// Most source addresses in one certificate.
pub const MAX_SOURCE_ADDRESSES: usize = 8;
/// Longest source address or CIDR.
pub const MAX_SOURCE_ADDRESS_BYTES: usize = 64;
/// Clock skew allowed before a certificate becomes valid, in seconds.
const BACKDATE_SECONDS: u64 = 60;

/// Why a certificate cannot be issued.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CertificateError {
    /// The principal does not name an agent.
    #[error("the principal does not name an agent")]
    Principal,
    /// The validity window is empty, negative or longer than 24 hours.
    #[error("the validity window is empty or longer than 24 hours")]
    Validity,
    /// Source addresses are missing, too many or not plain addresses.
    #[error("the source addresses are missing or invalid")]
    SourceAddress,
    /// The OpenSSH library refused to build or sign the certificate.
    #[error("the certificate could not be signed")]
    Signing(#[source] ssh_key::Error),
}

/// What the CA is asked to certify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateSpec {
    /// `nocterm-agent-<id>`: the only principal the certificate names.
    pub principal: String,
    /// The runner's ephemeral Ed25519 public key.
    pub subject_key: [u8; 32],
    /// Serial number, unique per CA.
    pub serial: u64,
    /// Start of validity, Unix milliseconds.
    pub valid_after_ms: i64,
    /// End of validity, Unix milliseconds.
    pub valid_before_ms: i64,
    /// Addresses or CIDRs the key may connect from (`source-address`).
    pub source_addresses: Vec<String>,
}

impl CertificateSpec {
    /// Checks the rules that make a certificate safe to sign.
    pub fn validate(&self) -> Result<(), CertificateError> {
        let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_');
        let named = self
            .principal
            .strip_prefix(PRINCIPAL_PREFIX)
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(plain));
        if !named || self.principal.len() > 64 {
            return Err(CertificateError::Principal);
        }
        let life = self.valid_before_ms.saturating_sub(self.valid_after_ms);
        if self.valid_after_ms < 0 || life <= 0 || life > MAX_VALIDITY_MS {
            return Err(CertificateError::Validity);
        }
        let address = |a: &String| {
            !a.is_empty()
                && a.len() <= MAX_SOURCE_ADDRESS_BYTES
                && a.chars()
                    .all(|c| c.is_ascii_hexdigit() || matches!(c, '.' | ':' | '/'))
        };
        if self.source_addresses.is_empty()
            || self.source_addresses.len() > MAX_SOURCE_ADDRESSES
            || !self.source_addresses.iter().all(address)
        {
            return Err(CertificateError::SourceAddress);
        }
        Ok(())
    }
}

/// An issued certificate as the one-line text of an `-cert.pub` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedCertificate {
    openssh: String,
}

impl IssuedCertificate {
    /// The certificate line, without a trailing newline.
    pub fn openssh(&self) -> &str {
        &self.openssh
    }

    /// Reads a certificate line back.
    pub fn parse(text: &str) -> Result<Certificate, KeyImportError> {
        Certificate::from_openssh(text.trim()).map_err(KeyImportError::Malformed)
    }
}

/// The user's SSH CA: an Ed25519 key that signs certificates for agents.
pub struct UserCa {
    seed: Zeroizing<[u8; 32]>,
    key: PrivateKey,
    public: PublicKeyText,
}

impl UserCa {
    /// A CA derived from `seed`. The caller supplies the entropy and stores the seed.
    pub fn from_seed(seed: [u8; 32]) -> Result<Self, KeyImportError> {
        let seed = Zeroizing::new(seed);
        let key = PrivateKey::new(
            KeypairData::Ed25519(Ed25519Keypair::from_seed(&seed)),
            "nocterm agent CA",
        )
        .map_err(KeyImportError::Malformed)?;
        let public = PublicKeyText::of(key.public_key())?;
        Ok(Self { seed, key, public })
    }

    /// The seed to keep in the vault; it is the whole private key.
    pub fn seed(&self) -> &[u8; 32] {
        &self.seed
    }

    /// The raw Ed25519 public key.
    pub fn public_key(&self) -> [u8; 32] {
        public_bytes(&self.public)
    }

    /// The public key as OpenSSH text.
    pub fn public_text(&self) -> &PublicKeyText {
        &self.public
    }

    /// The `SHA256:…` fingerprint of the CA key.
    pub fn fingerprint(&self) -> &str {
        self.public.fingerprint()
    }

    /// Certifies the subject key of `spec`.
    ///
    /// `nonce` is fresh entropy supplied by the caller.
    pub fn issue(
        &self,
        spec: &CertificateSpec,
        nonce: [u8; 32],
    ) -> Result<IssuedCertificate, CertificateError> {
        spec.validate()?;
        let seconds = |ms: i64| u64::try_from(ms / 1000).unwrap_or(0);
        let valid_after = seconds(spec.valid_after_ms).saturating_sub(BACKDATE_SECONDS);
        let subject = KeyData::Ed25519(Ed25519PublicKey(spec.subject_key));
        let signing = CertificateError::Signing;
        let mut builder = Builder::new(
            nonce.to_vec(),
            subject,
            valid_after,
            seconds(spec.valid_before_ms),
        )
        .map_err(signing)?;
        builder.serial(spec.serial).map_err(signing)?;
        builder.cert_type(CertType::User).map_err(signing)?;
        builder
            .key_id(format!("{}-{}", spec.principal, spec.serial))
            .map_err(signing)?;
        builder
            .valid_principal(spec.principal.clone())
            .map_err(signing)?;
        builder
            .critical_option("source-address", spec.source_addresses.join(","))
            .map_err(signing)?;
        // Only a terminal: no port, agent or X11 forwarding, and no user-rc.
        builder.extension("permit-pty", "").map_err(signing)?;
        let certificate = builder.sign(&self.key).map_err(signing)?;
        let openssh = certificate.to_openssh().map_err(signing)?;
        Ok(IssuedCertificate { openssh })
    }
}

/// The SHA-256 fingerprint OpenSSH prints for a raw Ed25519 public key.
pub fn fingerprint_of(public_key: &[u8; 32]) -> String {
    KeyData::Ed25519(Ed25519PublicKey(*public_key))
        .fingerprint(HashAlg::Sha256)
        .to_string()
}

fn public_bytes(public: &PublicKeyText) -> [u8; 32] {
    let key = ssh_key::PublicKey::from_openssh(public.as_str());
    match key.as_ref().map(|key| key.key_data()) {
        Ok(KeyData::Ed25519(key)) => key.0,
        _ => [0; 32],
    }
}

impl std::fmt::Debug for UserCa {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "UserCa({})", self.fingerprint())
    }
}

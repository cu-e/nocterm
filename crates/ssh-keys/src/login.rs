//! Checked SSH initial-host and exact public-key userauth proofs, never an arbitrary signer.
use crate::PublicKeyText;
use nocterm_session::Secret;
use signature::Signer as _;
use ssh_key::{
    Algorithm, PrivateKey, PublicKey, Signature,
    encoding::{Decode as _, Encode as _},
};

pub const MAX_LOGIN_PAYLOAD: usize = 4096;
pub const MAX_HOST_PROOF: usize = 1024;
pub const MAX_LOGIN_SIGNATURE: usize = 128;
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LoginProofError {
    #[error("SSH login proof is malformed or exceeds its bound")]
    Malformed,
    #[error("SSH login proof uses an unsupported algorithm or authentication method")]
    Unsupported,
    #[error("SSH login proof does not match the original consent")]
    Mismatch,
    #[error("the pinned host did not sign this initial SSH session identifier")]
    HostSignature,
    #[error("the selected SSH private key cannot sign this login")]
    PrivateKey,
}
/// The only public key algorithm accepted by the initial login proof contract.
pub fn login_key_from_blob(blob: &[u8]) -> Result<PublicKeyText, LoginProofError> {
    if blob.len() > MAX_HOST_PROOF {
        return Err(LoginProofError::Malformed);
    }
    let key = PublicKey::from_bytes(blob).map_err(|_| LoginProofError::Malformed)?;
    if key.algorithm() != Algorithm::Ed25519 {
        return Err(LoginProofError::Unsupported);
    }
    if key.to_bytes().map_err(|_| LoginProofError::Malformed)? != blob {
        return Err(LoginProofError::Malformed);
    }
    PublicKeyText::of(&key).map_err(|_| LoginProofError::Malformed)
}
pub fn login_key_blob(key: &PublicKeyText) -> Result<Vec<u8>, LoginProofError> {
    let key = PublicKey::from_openssh(key.as_str()).map_err(|_| LoginProofError::Malformed)?;
    if key.algorithm() != Algorithm::Ed25519 {
        return Err(LoginProofError::Unsupported);
    }
    key.to_bytes().map_err(|_| LoginProofError::Malformed)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitialHostProof {
    host_key: Vec<u8>,
    signature: Vec<u8>,
}
impl InitialHostProof {
    pub fn new(host_key: Vec<u8>, signature: Vec<u8>) -> Result<Self, LoginProofError> {
        if host_key.len() > MAX_HOST_PROOF || signature.len() > MAX_HOST_PROOF {
            return Err(LoginProofError::Malformed);
        }
        login_key_from_blob(&host_key)?;
        decode_signature(&signature)?;
        Ok(Self {
            host_key,
            signature,
        })
    }
    pub fn host_key_blob(&self) -> &[u8] {
        &self.host_key
    }
    pub fn signature_blob(&self) -> &[u8] {
        &self.signature
    }
    pub fn verify(
        &self,
        pinned: &PublicKeyText,
        session: &[u8; 32],
    ) -> Result<(), LoginProofError> {
        let key = PublicKey::from_bytes(&self.host_key).map_err(|_| LoginProofError::Malformed)?;
        if PublicKeyText::of(&key).map_err(|_| LoginProofError::Malformed)? != *pinned {
            return Err(LoginProofError::Mismatch);
        }
        signature::Verifier::verify(&key, session, &decode_signature(&self.signature)?)
            .map_err(|_| LoginProofError::HostSignature)
    }
}
/// Full, bounded RFC4252 userauth action. No unchecked payload can be signed.
#[derive(Clone, PartialEq, Eq)]
pub struct UserauthLogin {
    payload: Vec<u8>,
    session: [u8; 32],
    principal: String,
    key: PublicKeyText,
}
impl std::fmt::Debug for UserauthLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserauthLogin")
            .field("bytes", &self.payload.len())
            .finish_non_exhaustive()
    }
}
impl UserauthLogin {
    pub fn parse(payload: Vec<u8>) -> Result<Self, LoginProofError> {
        if payload.len() > MAX_LOGIN_PAYLOAD {
            return Err(LoginProofError::Malformed);
        }
        let mut reader = Reader(&payload);
        let session = reader
            .string()?
            .try_into()
            .map_err(|_| LoginProofError::Malformed)?;
        if reader.byte()? != 50 {
            return Err(LoginProofError::Unsupported);
        }
        let principal = std::str::from_utf8(reader.string()?)
            .map_err(|_| LoginProofError::Malformed)?
            .to_owned();
        if principal.is_empty() || principal.len() > 256 || principal.bytes().any(|b| b == 0) {
            return Err(LoginProofError::Malformed);
        }
        if reader.string()? != b"ssh-connection"
            || reader.string()? != b"publickey"
            || reader.byte()? != 1
            || reader.string()? != b"ssh-ed25519"
        {
            return Err(LoginProofError::Unsupported);
        }
        let key = login_key_from_blob(reader.string()?)?;
        if !reader.0.is_empty() {
            return Err(LoginProofError::Malformed);
        }
        Ok(Self {
            payload,
            session,
            principal,
            key,
        })
    }
    pub fn session_id(&self) -> &[u8; 32] {
        &self.session
    }
    pub fn principal(&self) -> &str {
        &self.principal
    }
    pub fn user_key(&self) -> &PublicKeyText {
        &self.key
    }
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginSignature {
    blob: Vec<u8>,
}
impl LoginSignature {
    pub fn new(blob: Vec<u8>) -> Result<Self, LoginProofError> {
        if blob.len() > MAX_LOGIN_SIGNATURE {
            return Err(LoginProofError::Malformed);
        }
        decode_signature(&blob)?;
        Ok(Self { blob })
    }
    pub fn as_blob(&self) -> &[u8] {
        &self.blob
    }
    pub fn verify(&self, login: &UserauthLogin) -> Result<(), LoginProofError> {
        let key =
            PublicKey::from_openssh(login.key.as_str()).map_err(|_| LoginProofError::Malformed)?;
        signature::Verifier::verify(&key, &login.payload, &decode_signature(&self.blob)?)
            .map_err(|_| LoginProofError::Mismatch)
    }
}
/// A device-only secret owner exposing exactly the validated login operation.
pub struct LoginSigningKey {
    key: PrivateKey,
}
impl LoginSigningKey {
    pub fn import(secret: &Secret, expected: &PublicKeyText) -> Result<Self, LoginProofError> {
        if secret.expose().len() > 1024 * 1024 {
            return Err(LoginProofError::PrivateKey);
        }
        let key =
            PrivateKey::from_openssh(secret.expose()).map_err(|_| LoginProofError::PrivateKey)?;
        if key.algorithm() != Algorithm::Ed25519
            || PublicKeyText::of(key.public_key()).map_err(|_| LoginProofError::PrivateKey)?
                != *expected
        {
            return Err(LoginProofError::Mismatch);
        }
        Ok(Self { key })
    }
    pub fn sign_login(&self, login: &UserauthLogin) -> Result<LoginSignature, LoginProofError> {
        if PublicKeyText::of(self.key.public_key()).map_err(|_| LoginProofError::PrivateKey)?
            != login.key
        {
            return Err(LoginProofError::Mismatch);
        }
        let sig: Signature = self
            .key
            .try_sign(&login.payload)
            .map_err(|_| LoginProofError::PrivateKey)?;
        LoginSignature::new(sig.encode_vec().map_err(|_| LoginProofError::Malformed)?)
    }
}
fn decode_signature(blob: &[u8]) -> Result<Signature, LoginProofError> {
    let mut reader = blob;
    let sig = Signature::decode(&mut reader).map_err(|_| LoginProofError::Malformed)?;
    if !reader.is_empty() {
        return Err(LoginProofError::Malformed);
    }
    if sig.algorithm() != Algorithm::Ed25519 {
        return Err(LoginProofError::Unsupported);
    }
    Ok(sig)
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8, LoginProofError> {
        let Some((b, rest)) = self.0.split_first() else {
            return Err(LoginProofError::Malformed);
        };
        self.0 = rest;
        Ok(*b)
    }
    fn string(&mut self) -> Result<&'a [u8], LoginProofError> {
        if self.0.len() < 4 {
            return Err(LoginProofError::Malformed);
        }
        let n = u32::from_be_bytes(
            self.0[..4]
                .try_into()
                .map_err(|_| LoginProofError::Malformed)?,
        ) as usize;
        self.0 = &self.0[4..];
        if n > self.0.len() {
            return Err(LoginProofError::Malformed);
        }
        let (s, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(s)
    }
}
#[cfg(test)]
mod tests;

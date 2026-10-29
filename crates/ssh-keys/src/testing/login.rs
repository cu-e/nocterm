//! Deterministic proof values shared by domain consumer behavior tests.
use crate::{
    KeyMaterial, PublicKeyText,
    login::{InitialHostProof, UserauthLogin},
};
use nocterm_session::Secret;
use signature::Signer as _;
use ssh_key::{PrivateKey, Signature, encoding::Encode as _};
pub struct LoginProofFixture {
    host: KeyMaterial,
    user: KeyMaterial,
}
impl Default for LoginProofFixture {
    fn default() -> Self {
        Self {
            host: KeyMaterial::ed25519_from_seed(&[1; 32], "host").unwrap(),
            user: KeyMaterial::ed25519_from_seed(&[2; 32], "user").unwrap(),
        }
    }
}
impl LoginProofFixture {
    pub fn host_key(&self) -> PublicKeyText {
        self.host.public_key()
    }
    pub fn user_key(&self) -> PublicKeyText {
        self.user.public_key()
    }
    pub fn user_secret(&self) -> Secret {
        self.user.to_secret()
    }
    pub fn host_proof(&self, session: [u8; 32]) -> InitialHostProof {
        let key = PrivateKey::from_openssh(self.host.to_secret().expose()).unwrap();
        let signature: Signature = key.try_sign(&session).unwrap();
        InitialHostProof::new(
            key.public_key().to_bytes().unwrap(),
            signature.encode_vec().unwrap(),
        )
        .unwrap()
    }
    pub fn login(&self, principal: &str, session: [u8; 32]) -> UserauthLogin {
        let key = ssh_key::PublicKey::from_openssh(self.user_key().as_str()).unwrap();
        let mut payload = Vec::new();
        string(&mut payload, &session);
        payload.push(50);
        string(&mut payload, principal.as_bytes());
        string(&mut payload, b"ssh-connection");
        string(&mut payload, b"publickey");
        payload.push(1);
        string(&mut payload, b"ssh-ed25519");
        string(&mut payload, &key.to_bytes().unwrap());
        UserauthLogin::parse(payload).unwrap()
    }
}
fn string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend((bytes.len() as u32).to_be_bytes());
    out.extend(bytes);
}

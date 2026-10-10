//! Entropy inputs supplied by the storage adapter. Production callers must supply
//! fresh independent values for every creation, password rotation and seal.
use crate::VaultKey;
pub struct SealNonces {
    wrap: [u8; 24],
    payload: [u8; 24],
}
impl SealNonces {
    pub fn new(wrap: [u8; 24], payload: [u8; 24]) -> Self {
        Self { wrap, payload }
    }
}
pub struct CreateMaterial {
    salt: [u8; 16],
    dek: VaultKey,
    vault_id: [u8; 16],
    nonces: SealNonces,
}
impl CreateMaterial {
    pub fn new(salt: [u8; 16], dek: VaultKey, vault_id: [u8; 16], nonces: SealNonces) -> Self {
        Self {
            salt,
            dek,
            vault_id,
            nonces,
        }
    }
}
pub struct RotationMaterial {
    salt: [u8; 16],
    nonces: SealNonces,
}
impl RotationMaterial {
    pub fn new(salt: [u8; 16], nonces: SealNonces) -> Self {
        Self { salt, nonces }
    }
}

impl SealNonces {
    pub(crate) fn into_parts(self) -> ([u8; 24], [u8; 24]) {
        (self.wrap, self.payload)
    }
}
impl CreateMaterial {
    pub(crate) fn into_parts(self) -> ([u8; 16], VaultKey, [u8; 16], SealNonces) {
        (self.salt, self.dek, self.vault_id, self.nonces)
    }
}
impl RotationMaterial {
    pub(crate) fn into_parts(self) -> ([u8; 16], SealNonces) {
        (self.salt, self.nonces)
    }
}

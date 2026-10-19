use crate::{VaultBinding, VaultError};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceRegistration {
    version: u8,
    binding: VaultBinding,
    token: Vec<u8>,
}
impl DeviceRegistration {
    pub fn new(binding: VaultBinding, token: Vec<u8>) -> Result<Self, VaultError> {
        if token.len() > 4096 {
            return Err(VaultError::Limit);
        }
        Ok(Self {
            version: 1,
            binding,
            token,
        })
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, VaultError> {
        if bytes.len() > 32768 {
            return Err(VaultError::Limit);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| VaultError::Format)?;
        if value.version != 1 || value.token.len() > 4096 {
            return Err(VaultError::Format);
        }
        Ok(value)
    }
    pub fn encode(&self) -> Result<Vec<u8>, VaultError> {
        serde_json::to_vec(self).map_err(|_| VaultError::Format)
    }
    pub fn binding(&self) -> VaultBinding {
        self.binding
    }
    pub fn token(&self) -> &[u8] {
        &self.token
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_preserves_binding_and_rejects_versions_and_oversized_tokens() {
        let binding = VaultBinding {
            vault_id: [1; 16],
            salt: [2; 16],
        };
        let value = DeviceRegistration::new(binding, vec![3, 4]).unwrap();
        let encoded = value.encode().unwrap();
        let restored = DeviceRegistration::parse(&encoded).unwrap();
        assert_eq!(restored.binding(), binding);
        assert_eq!(restored.token(), [3, 4]);
        assert!(matches!(
            DeviceRegistration::new(binding, vec![0; 4097]),
            Err(VaultError::Limit)
        ));
        assert!(matches!(
            DeviceRegistration::parse(&vec![0; 32769]),
            Err(VaultError::Limit)
        ));
        let mut invalid: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        invalid["version"] = 2.into();
        assert!(matches!(
            DeviceRegistration::parse(&serde_json::to_vec(&invalid).unwrap()),
            Err(VaultError::Format)
        ));
    }
}

use nocterm_session::CredentialId;
use nocterm_vault::{CreateMaterial, RotationMaterial, SealNonces, VaultError, VaultKey};
pub(crate) fn random<const N: usize>() -> Result<[u8; N], VaultError> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| VaultError::Randomness)?;
    Ok(bytes)
}
pub(crate) fn credential_id() -> Result<CredentialId, VaultError> {
    Ok(CredentialId::from_bytes(random()?))
}
pub(crate) fn nonces() -> Result<SealNonces, VaultError> {
    Ok(SealNonces::new(random()?, random()?))
}
pub(crate) fn create_material() -> Result<CreateMaterial, VaultError> {
    Ok(CreateMaterial::new(
        random()?,
        VaultKey::new(random()?),
        random()?,
        nonces()?,
    ))
}
pub(crate) fn rotation_material() -> Result<RotationMaterial, VaultError> {
    Ok(RotationMaterial::new(random()?, nonces()?))
}

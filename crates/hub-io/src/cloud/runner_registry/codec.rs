//! Versioned private account object layout; never a public Cloud wire codec.
use super::{ChatMaterialPins, RunnerRegistration, RunnerRegistryError};
use nocterm_cloud_model::{
    identity::{AccountScope, RunnerId, RunnerKeyEpoch},
    journal::JournalCatalogBinding,
};
use sha2::{Digest, Sha256};
pub(super) const MAX_PLAINTEXT: usize = 8 * 1024;
pub(super) const MAX_RUNNERS: usize = 16;
pub(super) const KIND: &str = "cloud-runner-registry";
pub(super) const OBJECT: &str = "trusted-runners-v1";

pub(super) fn context(scope: &AccountScope) -> Result<Vec<u8>, RunnerRegistryError> {
    let mut bytes = b"nocterm/private-runner-registry/v1".to_vec();
    for text in [
        scope.hub().instance().as_str(),
        scope.account().id().as_str(),
    ] {
        if text.is_empty() || text.len() > 128 {
            return Err(RunnerRegistryError::Capacity);
        }
        bytes.extend_from_slice(&(text.len() as u16).to_be_bytes());
        bytes.extend_from_slice(text.as_bytes());
    }
    bytes.extend_from_slice(scope.hub().authority().as_bytes());
    bytes.extend_from_slice(scope.account().key().as_bytes());
    Ok(bytes)
}
pub(super) fn collection_id(context: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"nocterm/private-runner-registry/collection/v1");
    hash.update(context);
    hash.update(KIND.as_bytes());
    let digest: [u8; 32] = hash.finalize().into();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Builder::from_custom_bytes(bytes)
        .into_uuid()
        .to_string()
}
pub(super) fn encode(
    context: &[u8],
    version: u64,
    rows: &[RunnerRegistration],
) -> Result<zeroize::Zeroizing<Vec<u8>>, RunnerRegistryError> {
    if rows.len() > MAX_RUNNERS || version != rows.len() as u64 {
        return Err(RunnerRegistryError::Format);
    }
    let mut size = context.len() + 9;
    for row in rows {
        size += 16 + 8 + 96 + 1 + row.label.len();
    }
    if size > MAX_PLAINTEXT {
        return Err(RunnerRegistryError::Capacity);
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(size));
    bytes.extend_from_slice(context);
    bytes.extend_from_slice(&version.to_be_bytes());
    bytes.push(rows.len() as u8);
    for row in rows {
        bytes.extend_from_slice(row.binding.runner().as_bytes());
        bytes.extend_from_slice(&row.binding.epoch().get().to_be_bytes());
        bytes.extend_from_slice(&row.pins.encryption);
        bytes.extend_from_slice(&row.pins.signing);
        bytes.extend_from_slice(&row.pins.proof);
        bytes.push(row.label.len() as u8);
        bytes.extend_from_slice(row.label.as_bytes());
    }
    Ok(bytes)
}
pub(super) fn decode(
    scope: &AccountScope,
    context: &[u8],
    version: u64,
    bytes: &[u8],
) -> Result<Vec<RunnerRegistration>, RunnerRegistryError> {
    if bytes.len() > MAX_PLAINTEXT {
        return Err(RunnerRegistryError::Capacity);
    }
    if !bytes.starts_with(context) {
        return Err(RunnerRegistryError::Binding);
    }
    let mut reader = Reader {
        bytes: &bytes[context.len()..],
    };
    let stored_version = u64::from_be_bytes(reader.array()?);
    let count = usize::from(reader.array::<1>()?[0]);
    if count == 0 || count > MAX_RUNNERS || stored_version != version || version != count as u64 {
        return Err(RunnerRegistryError::Format);
    }
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let id = RunnerId::from_bytes(reader.array()?);
        let epoch = RunnerKeyEpoch::new(u64::from_be_bytes(reader.array()?))
            .map_err(|_| RunnerRegistryError::Format)?;
        let pins = ChatMaterialPins {
            encryption: reader.array()?,
            signing: reader.array()?,
            proof: reader.array()?,
        };
        let length = usize::from(reader.array::<1>()?[0]);
        if length == 0 || length > 128 {
            return Err(RunnerRegistryError::Capacity);
        }
        let label =
            std::str::from_utf8(reader.take(length)?).map_err(|_| RunnerRegistryError::Format)?;
        let row = RunnerRegistration::new(
            JournalCatalogBinding::new(scope.clone(), id, epoch),
            pins,
            label.to_owned(),
        )?;
        if rows.last().is_some_and(|previous: &RunnerRegistration| {
            previous.binding.runner().as_bytes() >= row.binding.runner().as_bytes()
        }) {
            return Err(RunnerRegistryError::Format);
        }
        rows.push(row);
    }
    if !reader.bytes.is_empty() {
        return Err(RunnerRegistryError::Format);
    }
    Ok(rows)
}
struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], RunnerRegistryError> {
        let (value, rest) = self
            .bytes
            .split_at_checked(count)
            .ok_or(RunnerRegistryError::Format)?;
        self.bytes = rest;
        Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], RunnerRegistryError> {
        self.take(N)?
            .try_into()
            .map_err(|_| RunnerRegistryError::Format)
    }
}

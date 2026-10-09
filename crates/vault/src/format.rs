//! Version 1 of the vault file: key derivation, the record codec, encryption
//! and bounded reading of the file.

use super::*;

pub(crate) fn random<const N: usize>() -> Result<[u8; N], VaultError> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| VaultError::Randomness)?;
    Ok(bytes)
}
pub(crate) fn derive(password: &Secret, salt: &[u8; 16]) -> Result<VaultKey, VaultError> {
    if password.expose().len() > 4096 {
        return Err(VaultError::Password);
    }
    let params =
        Params::new(MEMORY_KIB, ITERATIONS, LANES, Some(32)).map_err(|_| VaultError::Format)?;
    let mut key = VaultKey::new([0; 32]);
    let mut memory = Zeroizing::new(vec![argon2::Block::default(); MEMORY_KIB as usize]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into_with_memory(
            password.expose().as_bytes(),
            salt,
            key.as_mut(),
            &mut *memory,
        )
        .map_err(|_| VaultError::Authentication)?;
    Ok(key)
}
pub(crate) fn cipher(key: &[u8; 32]) -> Result<XChaCha20Poly1305, VaultError> {
    XChaCha20Poly1305::new_from_slice(key).map_err(|_| VaultError::Format)
}

pub(crate) struct Header {
    pub(crate) salt: [u8; 16],
    pub(crate) vault_id: [u8; 16],
    pub(crate) wrap_nonce: [u8; 24],
    pub(crate) payload_nonce: [u8; 24],
}
impl Header {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, VaultError> {
        if !(HEADER_SIZE + WRAPPED_SIZE + 16..=MAX_FILE).contains(&bytes.len())
            || &bytes[..8] != MAGIC
            || bytes[8..12] != [0, 1, 1, 19]
        {
            return Err(VaultError::Format);
        }
        let u32_at = |offset| {
            u32::from_be_bytes(
                bytes[offset..offset + 4]
                    .try_into()
                    .expect("validated header length"),
            )
        };
        // Version 1 deliberately accepts exactly its documented KDF parameters;
        // hostile headers cannot raise memory, iterations or lanes before KDF.
        if u32_at(12) != MEMORY_KIB
            || u32_at(16) != ITERATIONS
            || u32_at(20) != LANES
            || u32_at(104) as usize != bytes.len() - HEADER_SIZE - WRAPPED_SIZE
        {
            return Err(VaultError::Format);
        }
        Ok(Self {
            salt: bytes[24..40].try_into().expect("fixed salt"),
            vault_id: bytes[40..56].try_into().expect("fixed id"),
            wrap_nonce: bytes[56..80].try_into().expect("fixed nonce"),
            payload_nonce: bytes[80..104].try_into().expect("fixed nonce"),
        })
    }
}

pub(crate) fn encode_records(records: &[Record]) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_PAYLOAD));
    bytes.extend_from_slice(b"CRD1");
    bytes.extend_from_slice(&(records.len() as u32).to_be_bytes());
    for record in records {
        let binding = Zeroizing::new(
            serde_json::to_vec(&record.info.binding).map_err(|_| VaultError::Format)?,
        );
        if binding.len() > 4096 {
            return Err(VaultError::Limit);
        }
        let id = record.info.id.to_string();
        let fields = [
            id.as_bytes(),
            record.info.label.as_bytes(),
            &binding,
            record.secret.expose().as_bytes(),
        ];
        for field in fields {
            if field.len() + 4 > MAX_PAYLOAD - bytes.len() {
                return Err(VaultError::Limit);
            }
            bytes.extend_from_slice(&(field.len() as u32).to_be_bytes());
            bytes.extend_from_slice(field);
        }
    }
    Ok(bytes)
}
pub(crate) fn decode_records(mut bytes: &[u8]) -> Result<Vec<Record>, VaultError> {
    fn take<'a>(bytes: &mut &'a [u8], length: usize) -> Result<&'a [u8], VaultError> {
        if length > bytes.len() {
            return Err(VaultError::Format);
        }
        let (value, rest) = bytes.split_at(length);
        *bytes = rest;
        Ok(value)
    }
    fn size(bytes: &mut &[u8]) -> Result<usize, VaultError> {
        Ok(
            u32::from_be_bytes(take(bytes, 4)?.try_into().map_err(|_| VaultError::Format)?)
                as usize,
        )
    }
    fn text<'a>(bytes: &mut &'a [u8], maximum: usize) -> Result<&'a str, VaultError> {
        let length = size(bytes)?;
        if length > maximum {
            return Err(VaultError::Limit);
        }
        std::str::from_utf8(take(bytes, length)?).map_err(|_| VaultError::Format)
    }
    if take(&mut bytes, 4)? != b"CRD1" {
        return Err(VaultError::Format);
    }
    let count = size(&mut bytes)?;
    if count > MAX_RECORDS {
        return Err(VaultError::Limit);
    }
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        let id = text(&mut bytes, 32)?
            .parse()
            .map_err(|_| VaultError::Format)?;
        let label = text(&mut bytes, 1024)?.to_owned();
        let binding =
            serde_json::from_str(text(&mut bytes, 4096)?).map_err(|_| VaultError::Format)?;
        let secret = Secret::new(text(&mut bytes, 65536)?.to_owned());
        records.push(Record {
            info: CredentialInfo { id, label, binding },
            secret,
        });
    }
    if !bytes.is_empty() {
        return Err(VaultError::Format);
    }
    Ok(records)
}
pub(crate) fn encrypt(state: &Unlocked) -> Result<Vec<u8>, VaultError> {
    let plaintext = encode_records(&state.records)?;
    let wrap_nonce: [u8; 24] = random()?;
    let payload_nonce: [u8; 24] = random()?;
    let mut bytes = Vec::with_capacity(HEADER_SIZE + WRAPPED_SIZE + plaintext.len() + 16);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&[0, 1, 1, 19]);
    for value in [MEMORY_KIB, ITERATIONS, LANES] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&state.salt);
    bytes.extend_from_slice(&state.vault_id);
    bytes.extend_from_slice(&wrap_nonce);
    bytes.extend_from_slice(&payload_nonce);
    bytes.extend_from_slice(&((plaintext.len() + 16) as u32).to_be_bytes());
    let wrapped = cipher(&state.kek)?
        .encrypt(
            &XNonce::from(wrap_nonce),
            Payload {
                msg: state.dek.as_ref(),
                aad: &bytes,
            },
        )
        .map_err(|_| VaultError::Authentication)?;
    bytes.extend_from_slice(&wrapped);
    let payload = cipher(&state.dek)?
        .encrypt(
            &XNonce::from(payload_nonce),
            Payload {
                msg: &plaintext,
                aad: &bytes,
            },
        )
        .map_err(|_| VaultError::Authentication)?;
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}
pub(crate) fn read_file(path: &Path) -> Result<Vec<u8>, VaultError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(VaultError::Format);
    }
    let mut file = File::open(path)?;
    if file.metadata()?.len() > MAX_FILE as u64 {
        return Err(VaultError::Limit);
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FILE {
        return Err(VaultError::Limit);
    }
    Ok(bytes)
}

//! The two package formats and how a code package carries its manifest.
//!
//! A declarative package is one TOML file. A code package is one WebAssembly module whose custom
//! section [`MANIFEST_SECTION`] holds the same TOML. The kind is read from the first four bytes,
//! never from a file name. The section walk below is a pure, bounded scan of the section
//! headers: it does not validate the module (the runtime does that after consent), and it never
//! reads past the buffer.

use thiserror::Error;

/// The first four bytes of a WebAssembly module.
const MAGIC: [u8; 4] = *b"\0asm";
/// The only module version this host reads.
const VERSION: [u8; 4] = [1, 0, 0, 0];
/// A LEB128 `u32` takes at most this many bytes.
const MAX_LEB_BYTES: usize = 5;

/// The custom section of a code package that holds its manifest TOML.
pub const MANIFEST_SECTION: &str = "nocterm.manifest";

/// What the bytes of a package are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageKind {
    /// A declarative package: TOML text.
    Manifest,
    /// A code package: a WebAssembly module.
    Module,
}

impl PackageKind {
    /// The file extension a package of this kind is stored under.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Manifest => "toml",
            Self::Module => "wasm",
        }
    }
}

/// The kind of `bytes`: a module if they start with the WebAssembly magic, else a manifest.
pub fn kind(bytes: &[u8]) -> PackageKind {
    if bytes.starts_with(&MAGIC) {
        PackageKind::Module
    } else {
        PackageKind::Manifest
    }
}

/// A module whose manifest section cannot be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum SectionError {
    #[error("the file is not a WebAssembly module of version 1")]
    Header,
    #[error("the module ends inside a section")]
    Truncated,
    #[error("a section size is not a valid 32-bit LEB128 number")]
    Leb,
    #[error("the module has no `{MANIFEST_SECTION}` section")]
    Missing,
    #[error("the module has more than one `{MANIFEST_SECTION}` section")]
    Duplicate,
}

/// The payload of the single `nocterm.manifest` custom section of a module.
///
/// Walks the section headers only. Each size is checked against the bytes that remain before
/// the section is entered, so a hostile size cannot make the walk read or allocate beyond the
/// buffer.
pub fn manifest_section(bytes: &[u8]) -> Result<&[u8], SectionError> {
    let rest = bytes
        .strip_prefix(&MAGIC)
        .and_then(|rest| rest.strip_prefix(&VERSION))
        .ok_or(SectionError::Header)?;
    let mut rest = rest;
    let mut found = None;
    while let Some((&id, after_id)) = rest.split_first() {
        let (size, after_size) = leb_u32(after_id)?;
        let size = usize::try_from(size).map_err(|_| SectionError::Truncated)?;
        if size > after_size.len() {
            return Err(SectionError::Truncated);
        }
        let (payload, next) = after_size.split_at(size);
        rest = next;
        if id != 0 {
            continue;
        }
        let (name_len, after_len) = leb_u32(payload)?;
        let name_len = usize::try_from(name_len).map_err(|_| SectionError::Truncated)?;
        if name_len > after_len.len() {
            return Err(SectionError::Truncated);
        }
        let (name, data) = after_len.split_at(name_len);
        if name == MANIFEST_SECTION.as_bytes() && found.replace(data).is_some() {
            return Err(SectionError::Duplicate);
        }
    }
    found.ok_or(SectionError::Missing)
}

/// Reads one LEB128 `u32` and returns it with the bytes after it.
fn leb_u32(bytes: &[u8]) -> Result<(u32, &[u8]), SectionError> {
    let mut value = 0u32;
    for (index, &byte) in bytes.iter().take(MAX_LEB_BYTES).enumerate() {
        let payload = u32::from(byte & 0x7f);
        let shift = 7 * index as u32;
        // The fifth byte holds bits 28..32 only: anything above them overflows a u32.
        if index == MAX_LEB_BYTES - 1 && (byte & 0x80 != 0 || payload > 0x0f) {
            return Err(SectionError::Leb);
        }
        value |= payload << shift;
        if byte & 0x80 == 0 {
            return Ok((value, &bytes[index + 1..]));
        }
    }
    if bytes.len() < MAX_LEB_BYTES {
        Err(SectionError::Truncated)
    } else {
        Err(SectionError::Leb)
    }
}

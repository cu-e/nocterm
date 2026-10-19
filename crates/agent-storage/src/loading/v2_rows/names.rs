//! Fully validated fixed-prefix names; unknown long names never allocate strings.
use crate::loading::string_reader::{JsonStringReader, StringReadError};
use std::io::Read;
pub(super) struct Name {
    bytes: [u8; 16],
    used: usize,
}
impl Name {
    pub(super) fn as_str(&self) -> Result<&str, StringReadError> {
        std::str::from_utf8(&self.bytes[..self.used]).map_err(StringReadError::Utf8)
    }
}
pub(super) fn read(source: &mut dyn Read) -> Result<Option<Name>, StringReadError> {
    let mut cursor = JsonStringReader::new(source, nocterm_ai::history::MAX_FILE_BYTES as usize)?;
    let mut name = Name {
        bytes: [0; 16],
        used: 0,
    };
    let mut overflow = false;
    while let Some(fragment) = cursor.next_fragment()? {
        for byte in fragment.as_bytes() {
            if let Some(slot) = name.bytes.get_mut(name.used) {
                *slot = *byte;
                name.used += 1;
            } else {
                overflow = true;
            }
        }
    }
    Ok((!overflow).then_some(name))
}

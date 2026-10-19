//! Bounded reads and checked chat paths.
mod held_file;
pub(crate) mod reader_task;
mod string_reader;
use nocterm_ai::{
    history::{HistoryCodecError, MAX_FILE_BYTES},
    storage::AgentStorageError,
};
use nocterm_core::SharedError;
use std::{
    fs::File,
    io::Read as _,
    path::{Path, PathBuf},
};
pub(super) fn file(dir: &Path, id: &str) -> Result<PathBuf, AgentStorageError> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(AgentStorageError::InvalidId(id.into()));
    }
    Ok(dir.join(format!("{id}.json")))
}
pub(super) fn read(path: &Path, id: &str) -> Result<Vec<u8>, AgentStorageError> {
    let opened = File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AgentStorageError::NotFound(id.into())
        } else {
            read_error(path, error)
        }
    })?;
    if opened
        .metadata()
        .map_err(|error| read_error(path, error))?
        .len()
        > MAX_FILE_BYTES
    {
        return Err(HistoryCodecError::EncodedLimit.into());
    }
    let mut bytes = Vec::new();
    opened
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| read_error(path, error))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(HistoryCodecError::EncodedLimit.into());
    }
    Ok(bytes)
}
fn read_error(path: &Path, source: std::io::Error) -> AgentStorageError {
    AgentStorageError::Read {
        path: path.to_owned(),
        source: SharedError::from_error(source),
    }
}

#[cfg(test)]
mod legacy_validation;

mod literal_reader;

mod json_fields;
mod json_frames;
#[cfg(test)]
mod v2_plan;

#[cfg(test)]
mod v2_rows;

pub(crate) mod native;

//! Bounded reads retain the original opened source and its supplied file facts.
use crate::namespace::Partition;
use nocterm_ai::{
    history::{self, ContextId, FileStamp, HistoryCodecError, MAX_FILE_BYTES},
    storage::{AgentStorageError, require_chat_scope},
};
use nocterm_session::local_fs::{LocalFsError, LocalSource};

pub(super) struct Input {
    source: Box<dyn LocalSource>,
    stamp: FileStamp,
}
impl Input {
    pub(super) fn open(
        partition: &Partition,
        id: &str,
        extension: &str,
    ) -> Result<Self, AgentStorageError> {
        let source = partition
            .directory()
            .open_source(&partition.name(id, extension)?)
            .map_err(|error| match error {
                LocalFsError::NotFound(_) => AgentStorageError::NotFound(id.into()),
                error => error.into(),
            })?;
        let metadata = source.metadata()?;
        let bytes = metadata.size.ok_or(HistoryCodecError::EncodedLimit)?;
        if bytes > MAX_FILE_BYTES {
            return Err(HistoryCodecError::EncodedLimit.into());
        }
        Ok(Self {
            source,
            stamp: FileStamp {
                bytes,
                modified_nanos: u128::from(metadata.modified.unwrap_or(0)) * 1_000_000_000,
            },
        })
    }
    pub(super) fn stamp(&self) -> FileStamp {
        self.stamp
    }
    fn read(&mut self, limit: usize) -> Result<Vec<u8>, AgentStorageError> {
        let mut bytes = Vec::new();
        self.fill(&mut bytes, limit)?;
        Ok(bytes)
    }
    fn fill(&mut self, bytes: &mut Vec<u8>, limit: usize) -> Result<(), AgentStorageError> {
        while bytes.len() < limit {
            let chunk = self.source.read((limit - bytes.len()).min(32 * 1024))?;
            if chunk.is_empty() {
                break;
            }
            bytes.extend(chunk);
        }
        Ok(())
    }
    /// Admit a scoped header before continuing this same held source. A refused
    /// prefix never causes a full foreign payload read or a second pathname open.
    pub(super) fn scoped_record(
        mut self,
        id: &str,
        scope: ContextId,
    ) -> Result<Vec<u8>, AgentStorageError> {
        let mut bytes = self.read(history::MAX_HEADER_BYTES)?;
        let header = history::decode_catalog_header(&bytes, id)?;
        require_chat_scope(scope, header.scope())?;
        self.fill(&mut bytes, MAX_FILE_BYTES as usize + 1)?;
        self.finish(bytes)
    }
    pub(super) fn prefix(mut self) -> Result<Vec<u8>, AgentStorageError> {
        let bytes = self.read(nocterm_ai::history::MAX_HEADER_BYTES)?;
        self.source.close()?;
        Ok(bytes)
    }
    pub(super) fn all(mut self) -> Result<Vec<u8>, AgentStorageError> {
        let bytes = self.read(MAX_FILE_BYTES as usize + 1)?;
        self.finish(bytes)
    }
    fn finish(self, bytes: Vec<u8>) -> Result<Vec<u8>, AgentStorageError> {
        self.source.close()?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(HistoryCodecError::EncodedLimit.into());
        }
        Ok(bytes)
    }
}

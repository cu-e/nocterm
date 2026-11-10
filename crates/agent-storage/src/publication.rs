//! Scoped private staging uses the supplied held-directory publication lifecycle.
use crate::namespace::Partition;
use nocterm_ai::storage::AgentStorageError;
use nocterm_session::fs::UploadMode;

pub(super) fn publish(
    partition: &Partition,
    id: &str,
    extension: &str,
    bytes: &[u8],
) -> Result<(), AgentStorageError> {
    let name = partition.name(id, extension)?;
    let mut stage = partition.directory().stage()?;
    let result = (|| {
        for chunk in bytes.chunks(32 * 1024) {
            stage.write(chunk)?;
        }
        stage.sync()?;
        stage.publish(&name, UploadMode::Replace)
    })();
    match result {
        Ok(_) => Ok(()), // Committed durability warnings cannot become failed writes.
        Err(operation) => match stage.cancel() {
            Ok(()) => Err(operation.into()),
            Err(cleanup) => Err(AgentStorageError::StageCleanup {
                operation: nocterm_core::SharedError::from_error(operation),
                cleanup: nocterm_core::SharedError::from_error(cleanup),
            }),
        },
    }
}

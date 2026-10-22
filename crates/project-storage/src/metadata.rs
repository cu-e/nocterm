use std::time::{SystemTime, UNIX_EPOCH};

use nocterm_project_model::{
    ProjectId, ProjectIdentity, ProjectMetadataError, ProjectMetadataSource,
};

#[derive(Default)]
pub struct NativeProjectMetadata;

impl ProjectMetadataSource for NativeProjectMetadata {
    fn new_identity(&self) -> Result<ProjectIdentity, ProjectMetadataError> {
        allocate_identity(self.timestamp(), getrandom::fill)
    }

    fn timestamp(&self) -> u64 {
        timestamp_at(SystemTime::now())
    }
}

fn allocate_identity(
    created: u64,
    fill: impl FnOnce(&mut [u8]) -> Result<(), getrandom::Error>,
) -> Result<ProjectIdentity, ProjectMetadataError> {
    let mut bytes = [0; 16];
    fill(&mut bytes).map_err(ProjectMetadataError::entropy)?;
    Ok(ProjectIdentity {
        id: ProjectId::from_bytes(bytes),
        created,
    })
}

fn timestamp_at(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn native_timestamp_uses_epoch_seconds_and_preserves_the_pre_epoch_fallback() {
        assert_eq!(timestamp_at(UNIX_EPOCH), 0);
        assert_eq!(timestamp_at(UNIX_EPOCH + Duration::from_millis(1999)), 1);
        assert_eq!(timestamp_at(UNIX_EPOCH - Duration::from_secs(1)), 0);
    }

    #[test]
    fn unavailable_native_entropy_retains_its_original_concrete_error() {
        use std::error::Error as _;
        let error = allocate_identity(123, |_| Err(getrandom::Error::UNSUPPORTED)).unwrap_err();
        assert!(matches!(error, ProjectMetadataError::Entropy(_)));
        let original = error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<getrandom::Error>()
            .unwrap();
        assert_eq!(*original, getrandom::Error::UNSUPPORTED);
    }
}

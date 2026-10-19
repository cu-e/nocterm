//! Requested native path conversion overlap for pinned Unix std/tempfile paths.
use super::super::PathAdmissionError;
use std::path::Path;

/// Exact maximum simultaneous temporary CString bytes in the Unix stage path.
///
/// Pinned std1.98.0 uses stack384 for open/remove_file; tempfile3.27 delegates
/// persist to rustix1.1.5, whose path arguments each use stack256. Rename holds
/// both argument conversions together. Native allocator/RSS and unrelated error
/// payloads are separate; Windows UTF16/native full-path growth is not modeled.
#[cfg(unix)]
pub fn publication_native_scratch_bytes(
    destination: &Path,
    stage: &Path,
) -> Result<usize, PathAdmissionError> {
    fn cstring(path: &Path, stack: usize) -> Result<usize, PathAdmissionError> {
        let bytes = path.as_os_str().as_encoded_bytes().len();
        if bytes < stack {
            Ok(0)
        } else {
            bytes.checked_add(1).ok_or(PathAdmissionError::Arithmetic)
        }
    }
    let create_and_cleanup = cstring(stage, 384)?;
    let rename = cstring(stage, 256)?
        .checked_add(cstring(destination, 256)?)
        .ok_or(PathAdmissionError::Arithmetic)?;
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let directory_sync = cstring(parent, 384)?;
    Ok(create_and_cleanup.max(rename).max(directory_sync))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn native_path_scratch_tracks_both_rename_conversions_and_std_thresholds() {
        for (destination, stage, expected) in [
            (255, 255, 0),
            (256, 255, 257),
            (256, 256, 514),
            (383, 383, 768),
            (384, 384, 770),
            (500, 900, 1402),
        ] {
            let destination = "d".repeat(destination);
            let stage = "s".repeat(stage);
            assert_eq!(
                publication_native_scratch_bytes(Path::new(&destination), Path::new(&stage))
                    .unwrap(),
                expected
            );
        }
    }
}

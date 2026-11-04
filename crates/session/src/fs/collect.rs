//! Small-document collection from an already admitted remote stream.
use super::{FsError, RemoteDownload};

/// A bounded document read failed before a complete response was available.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DownloadReadError {
    #[error("could not read remote document: {0}")]
    Read(#[source] FsError),
    #[error("remote document exceeds {limit} bytes")]
    TooLarge { limit: usize },
    #[error("reader returned {returned} bytes when requested {requested}")]
    InvalidChunk { requested: usize, returned: usize },
}

/// Collects at most `limit` bytes, reading one extra byte to distinguish EOF
/// from overflow. Chunk requests are clamped to 1..=32 KiB. The caller retains
/// the stream and decides how to handle close failures after success or error.
pub async fn read_bounded(
    reader: &mut dyn RemoteDownload,
    limit: usize,
    chunk: usize,
) -> Result<Vec<u8>, DownloadReadError> {
    let chunk = chunk.clamp(1, 32 * 1024);
    let mut bytes = Vec::new();
    loop {
        let requested = limit
            .saturating_sub(bytes.len())
            .saturating_add(1)
            .min(chunk);
        let data = reader
            .read(requested)
            .await
            .map_err(DownloadReadError::Read)?;
        if data.len() > requested {
            return Err(DownloadReadError::InvalidChunk {
                requested,
                returned: data.len(),
            });
        }
        if data.is_empty() {
            return Ok(bytes);
        }
        if data.len() > limit.saturating_sub(bytes.len()) {
            return Err(DownloadReadError::TooLarge { limit });
        }
        bytes.extend(data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        RemoteFs as _,
        testing::{FsOperation, InMemoryFs},
    };

    #[test]
    fn bounded_documents_keep_exact_limits_and_leave_close_to_the_caller() {
        futures::executor::block_on(async {
            let fs = InMemoryFs::default();
            for (bytes, limit, succeeds) in [
                (b"exact".as_slice(), 5, true),
                (b"extra!".as_slice(), 5, false),
                (b"".as_slice(), 0, true),
                (b"x".as_slice(), 0, false),
            ] {
                fs.seed_file("/document", bytes).unwrap();
                let mut reader = fs.download("/document").await.unwrap();
                let result = read_bounded(reader.as_mut(), limit, 2).await;
                if succeeds {
                    assert_eq!(result.unwrap(), bytes);
                } else {
                    assert_eq!(result.unwrap_err(), DownloadReadError::TooLarge { limit });
                }
                assert_eq!(fs.activity().live_downloads, 1);
                reader.close().await.unwrap();
                assert_eq!(fs.activity().live_downloads, 0);
            }
            fs.override_next_read("/document", vec![0; 4]).unwrap();
            let mut reader = fs.download("/document").await.unwrap();
            assert_eq!(
                read_bounded(reader.as_mut(), 10, 3).await.unwrap_err(),
                DownloadReadError::InvalidChunk {
                    requested: 3,
                    returned: 4
                }
            );
            fs.fail_next(FsOperation::Read, "/document", FsError::NotConnected);
            assert_eq!(
                read_bounded(reader.as_mut(), 10, 3).await.unwrap_err(),
                DownloadReadError::Read(FsError::NotConnected)
            );
        });
    }
}

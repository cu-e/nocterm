//! Bounded streaming downloads.
use super::*;
/// SFTP v3 cannot atomically express O_NOFOLLOW. Reject the final symlink
/// before open and verify the opened handle is regular; this still cannot
/// prove path identity against a concurrent remote rename/symlink swap.
pub(super) async fn start_download(
    sftp: &Sftp,
    downloads: &Downloads,
    source: String,
    fs: SshFs,
) -> Result<Box<dyn RemoteDownload>, FsError> {
    let metadata = stat(sftp, &source)
        .await?
        .ok_or_else(|| FsError::NotFound {
            path: source.clone(),
        })?;
    if metadata.kind != EntryKind::File || metadata.is_symlink {
        return Err(FsError::Unsupported(format!(
            "{source}: only regular files can be downloaded"
        )));
    }
    let id = Uuid::new_v4();
    let download = Arc::new(Mutex::new(Download {
        source: source.clone(),
        handle: None,
        offset: 0,
        eof: false,
    }));
    {
        let mut registry = downloads.lock().await;
        if registry.len() >= MAX_OPEN_DOWNLOADS {
            return Err(FsError::Other(
                "too many open downloads; close an existing reader".into(),
            ));
        }
        registry.insert(id, download.clone());
    }
    // Register ownership before awaiting OPEN, so session shutdown owns any
    // handle acquired while a request is in progress.
    let result = async {
        let handle = sftp
            .raw
            .open(&source, OpenFlags::READ, FileAttributes::empty())
            .await
            .map_err(|error| fs_error(error, &source))?
            .handle;
        download.lock().await.handle = Some(handle.clone());
        let attrs = sftp
            .raw
            .fstat(&handle)
            .await
            .map_err(|error| fs_error(error, &source))?
            .attrs;
        if attrs.file_type() != FileType::File {
            return Err(FsError::Unsupported(format!(
                "{source}: opened object is not a regular file"
            )));
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => Ok(Box::new(Reader { id, fs })),
        Err(error) => {
            let _ = close_download(sftp, downloads, id).await;
            Err(error)
        }
    }
}
pub(super) async fn read(
    sftp: &Sftp,
    downloads: &Downloads,
    id: Uuid,
    max_bytes: usize,
) -> Result<Vec<u8>, FsError> {
    let max_bytes = max_bytes.min(MAX_CHUNK);
    if max_bytes == 0 {
        return Err(FsError::Other("download read size must be positive".into()));
    }
    let download = downloads
        .lock()
        .await
        .get(&id)
        .cloned()
        .ok_or(FsError::NotConnected)?;
    let mut download = download.lock().await;
    let handle = download.handle.as_ref().ok_or(FsError::NotConnected)?;
    if download.eof {
        return Ok(Vec::new());
    }
    match sftp
        .raw
        .read(handle, download.offset, max_bytes.min(MAX_CHUNK) as u32)
        .await
    {
        Ok(data) => {
            if data.data.is_empty() || data.data.len() > max_bytes {
                return Err(FsError::Other(format!(
                    "{}: invalid SFTP read length",
                    download.source
                )));
            }
            download.offset = download
                .offset
                .checked_add(data.data.len() as u64)
                .ok_or_else(|| FsError::Other("download offset overflow".into()))?;
            Ok(data.data)
        }
        Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
            download.eof = true;
            Ok(Vec::new())
        }
        Err(error) => Err(fs_error(error, &download.source)),
    }
}
pub(super) async fn close_download(
    sftp: &Sftp,
    downloads: &Downloads,
    id: Uuid,
) -> Result<(), FsError> {
    let Some(download) = downloads.lock().await.remove(&id) else {
        return Ok(());
    };
    let mut download = download.lock().await;
    if let Some(handle) = download.handle.take() {
        sftp.raw
            .close(handle)
            .await
            .map_err(|error| fs_error(error, &download.source))?;
    }
    Ok(())
}

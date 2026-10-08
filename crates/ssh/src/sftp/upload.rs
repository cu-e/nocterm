//! Uploads through a temporary file that replaces the destination on finish.
use super::*;
pub(super) fn admit_upload(
    registry: &mut HashMap<Uuid, Arc<Mutex<Upload>>>,
    id: Uuid,
    upload: Arc<Mutex<Upload>>,
) -> Result<(), FsError> {
    if registry.len() >= MAX_OPEN_UPLOADS {
        return Err(FsError::Other(
            "too many open uploads; close an existing writer".into(),
        ));
    }
    registry.insert(id, upload);
    Ok(())
}

pub(super) async fn start_upload(
    sftp: &Sftp,
    uploads: &Uploads,
    destination: String,
    mode: UploadMode,
    fs: SshFs,
) -> Result<Box<dyn RemoteUpload>, FsError> {
    if mode == UploadMode::Replace && !sftp.atomic_replace {
        return Err(FsError::Unsupported(
            "this server cannot atomically replace files; choose Rename or Skip".into(),
        ));
    }
    if let Some(existing) = stat(sftp, &destination).await?
        && (mode == UploadMode::Create || existing.kind != EntryKind::File || existing.is_symlink)
    {
        return Err(FsError::AlreadyExists { path: destination });
    }
    let id = Uuid::new_v4();
    let temporary = path::join(path::parent(&destination), &format!(".nocterm-{id}.part"));
    let upload = Arc::new(Mutex::new(Upload {
        temporary: temporary.clone(),
        destination,
        handle: None,
        offset: 0,
        mode,
        failed: false,
    }));
    {
        let mut registry = uploads.lock().await;
        admit_upload(&mut registry, id, upload.clone())?;
    }
    let mut attrs = FileAttributes::empty();
    attrs.permissions = Some(0o600);
    let opened = sftp
        .raw
        .open(
            &temporary,
            OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            attrs,
        )
        .await;
    match opened {
        Ok(handle) => {
            upload.lock().await.handle = Some(handle.handle);
            Ok(Box::new(Writer { id, fs }))
        }
        Err(e) => {
            uploads.lock().await.remove(&id);
            Err(fs_error(e, &temporary))
        }
    }
}
pub(super) async fn get_upload(uploads: &Uploads, id: Uuid) -> Result<Arc<Mutex<Upload>>, FsError> {
    uploads
        .lock()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(|| FsError::Other("upload already finished or cancelled".into()))
}
pub(super) fn validate_batch(chunks: &[Vec<u8>]) -> Result<(), FsError> {
    if chunks.len() > MAX_WRITE_BATCH || chunks.iter().any(|chunk| chunk.len() > MAX_CHUNK) {
        return Err(FsError::Other("upload batch exceeds 8 × 32 KiB".into()));
    }
    Ok(())
}
pub(super) async fn write_batch(
    sftp: &Sftp,
    uploads: &Uploads,
    id: Uuid,
    chunks: Vec<Vec<u8>>,
) -> Result<(), FsError> {
    validate_batch(&chunks)?;
    let upload = get_upload(uploads, id).await?;
    // One writer owns the complete contiguous offset range until every
    // acknowledgement arrives. Other writes and finish cannot overtake it.
    let mut upload = upload.lock().await;
    if upload.failed {
        return Err(FsError::Other(
            "upload failed; cancel and start again".into(),
        ));
    }
    let handle = upload.handle.clone().ok_or(FsError::NotConnected)?;
    let mut end = upload.offset;
    let mut writes = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let offset = end;
        let Some(next) = end.checked_add(chunk.len() as u64) else {
            upload.failed = true;
            return Err(FsError::Other("upload offset overflow".into()));
        };
        end = next;
        writes.push(sftp.raw.write(handle.clone(), offset, chunk));
    }
    // Unlike try_join_all, this drains all replies even after one fails.
    // Never publish a partial batch or release the writer with pending ACKs.
    let replies = futures::future::join_all(writes).await;
    if let Some(error) = replies.into_iter().find_map(Result::err) {
        upload.failed = true;
        return Err(fs_error(error, &upload.destination));
    }
    upload.offset = end;
    Ok(())
}
pub(super) async fn finish(sftp: &Sftp, uploads: &Uploads, id: Uuid) -> Result<String, FsError> {
    let upload = get_upload(uploads, id).await?;
    let mut upload = upload.lock().await;
    if upload.failed {
        return Err(FsError::Other("failed upload cannot be published".into()));
    }
    if let Some(handle) = upload.handle.take() {
        sftp.raw
            .close(handle)
            .await
            .map_err(|e| fs_error(e, &upload.temporary))?;
    }
    let result = if upload.mode == UploadMode::Replace {
        let mut data = Vec::new();
        for path in [&upload.temporary, &upload.destination] {
            data.extend_from_slice(&(path.len() as u32).to_be_bytes());
            data.extend_from_slice(path.as_bytes());
        }
        match sftp.raw.extended("posix-rename@openssh.com", data).await {
            Ok(Packet::Status(status)) if status.status_code == StatusCode::Ok => Ok(()),
            Ok(Packet::Status(status)) => Err(SftpError::Status(status)),
            Ok(_) => Err(SftpError::UnexpectedPacket),
            Err(e) => Err(e),
        }
    } else {
        sftp.raw
            .rename(&upload.temporary, &upload.destination)
            .await
            .map(|_| ())
    };
    if let Err(error) = result {
        if upload.mode == UploadMode::Create && stat(sftp, &upload.destination).await?.is_some() {
            return Err(FsError::AlreadyExists {
                path: upload.destination.clone(),
            });
        }
        return Err(fs_error(error, &upload.destination));
    }
    uploads.lock().await.remove(&id);
    Ok(upload.destination.clone())
}
pub(super) async fn cancel(sftp: &Sftp, uploads: &Uploads, id: Uuid) -> Result<(), FsError> {
    let Some(upload) = uploads.lock().await.remove(&id) else {
        return Ok(());
    };
    let mut upload = upload.lock().await;
    if let Some(handle) = upload.handle.take() {
        let _ = sftp.raw.close(handle).await;
    }
    sftp.raw
        .remove(&upload.temporary)
        .await
        .map(|_| ())
        .map_err(|e| fs_error(e, &upload.temporary))
}

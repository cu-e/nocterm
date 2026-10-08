//! The session-owned SFTP worker: request admission, dispatch and shutdown.
use super::*;
/// Worker tasks are owned by the session. Shutdown cancels requests, then
/// removes only this session's partial files before SSH disconnect.
pub(crate) async fn serve(
    handle: Arc<Handle<Client>>,
    mut requests: FsRequests,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let mut opened: Option<Result<Arc<Sftp>, FsError>> = None;
    let uploads: Uploads = Arc::default();
    let downloads: Downloads = Arc::default();
    let mut workers = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => break,
            Some(_) = workers.join_next(), if !workers.is_empty() => {},
            Some(cleanup) = requests.cleanup.recv(), if workers.len() < CONCURRENT_REQUESTS => {
                if let Some(Ok(sftp)) = &opened {
                    let uploads = uploads.clone(); let downloads = downloads.clone(); let sftp = sftp.clone();
                    workers.spawn(async move { match cleanup {
                        Cleanup::Upload(id) => { let _ = cancel(&sftp, &uploads, id).await; }
                        Cleanup::Download(id) => { let _ = close_download(&sftp, &downloads, id).await; }
                    } });
                }
            },
            request = requests.requests.recv(), if workers.len() < CONCURRENT_REQUESTS => {
                let Some(request) = request else { break; };
                let sftp = match &opened {
                    Some(result) => result.clone(),
                    None => opened.insert(open(&handle).await.map(Arc::new)).clone(),
                };
                let uploads = uploads.clone();
                let downloads = downloads.clone();
                let sender = requests.sender.clone();
                workers.spawn(async move { dispatch(request, sftp, uploads, downloads, sender).await });
            }
        }
    }
    workers.abort_all();
    while workers.join_next().await.is_some() {}
    if let Some(Ok(sftp)) = opened {
        let ids: Vec<_> = uploads.lock().await.keys().copied().collect();
        for id in ids {
            let _ = cancel(&sftp, &uploads, id).await;
        }
        let ids: Vec<_> = downloads.lock().await.keys().copied().collect();
        for id in ids {
            let _ = close_download(&sftp, &downloads, id).await;
        }
        let _ = sftp.raw.close_session();
    }
}
pub(super) async fn dispatch(
    request: FsRequest,
    result: Result<Arc<Sftp>, FsError>,
    uploads: Uploads,
    downloads: Downloads,
    fs: SshFs,
) {
    match request {
        FsRequest::Home(reply) => {
            let _ = reply.send(match result {
                Ok(s) => home(&s).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::ReadDir(path, reply) => {
            let _ = reply.send(match result {
                Ok(s) => read_dir(&s, &path).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Stat(path, reply) => {
            let _ = reply.send(match result {
                Ok(s) => stat(&s, &path).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Metadata(path, reply) => {
            let _ = reply.send(match result {
                Ok(s) => metadata(&s, &path).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Rename(old, new, reply) => {
            let _ = reply.send(match result {
                Ok(s) => rename(&s, &old, &new).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::RemoveFile(path, reply) => {
            let _ = reply.send(match result {
                Ok(s) => remove_file(&s, &path).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::RemoveDir(path, reply) => {
            let _ = reply.send(match result {
                Ok(s) => remove_dir(&s, &path).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::SetPermissions(path, permissions, reply) => {
            let _ = reply.send(match result {
                Ok(s) => set_permissions(&s, &path, permissions).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::CreateDir(path, reply) => {
            let _ = reply.send(match result {
                Ok(s) => create_dir(&s, &path).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Upload(path, mode, reply) => {
            let _ = reply.send(match result {
                Ok(s) => start_upload(&s, &uploads, path, mode, fs).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::WriteBatch(id, chunks, reply) => {
            let _ = reply.send(match result {
                Ok(s) => write_batch(&s, &uploads, id, chunks).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Finish(id, reply) => {
            let _ = reply.send(match result {
                Ok(s) => finish(&s, &uploads, id).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Cancel(id, reply) => {
            let _ = reply.send(match result {
                Ok(s) => cancel(&s, &uploads, id).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Download(path, reply) => {
            let _ = reply.send(match result {
                Ok(s) => start_download(&s, &downloads, path, fs).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Read(id, max_bytes, reply) => {
            let _ = reply.send(match result {
                Ok(s) => read(&s, &downloads, id, max_bytes).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::CloseDownload(id, reply) => {
            let _ = reply.send(match result {
                Ok(s) => close_download(&s, &downloads, id).await,
                Err(e) => Err(e),
            });
        }
    }
}
pub(super) async fn open(handle: &Handle<Client>) -> Result<Sftp, FsError> {
    let unsupported = |e: &dyn std::fmt::Display| FsError::Unsupported(e.to_string());
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| unsupported(&e))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| unsupported(&e))?;
    let raw = RawSftpSession::new(channel.into_stream());
    let version = raw.init().await.map_err(|e| unsupported(&e))?;
    Ok(Sftp {
        raw,
        atomic_replace: version
            .extensions
            .get("posix-rename@openssh.com")
            .is_some_and(|v| v == "1"),
    })
}

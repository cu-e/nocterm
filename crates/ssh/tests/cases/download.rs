use super::*;
use nocterm_session::fs::{FsError, UploadMode};

#[tokio::test]
async fn download_streams_bounded_exact_bytes_and_explicit_eof() {
    let Some(sshd) = Sshd::start() else { return };
    let data: Vec<_> = (0..150_003).map(|n| (n % 251) as u8).collect();
    let source = sshd.root().join("download.bin");
    fs::write(&source, &data).unwrap();
    let empty = sshd.root().join("empty.bin");
    fs::write(&empty, []).unwrap();
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let remote = session.fs().unwrap();
    let mut reader = remote.download(source.to_str().unwrap()).await.unwrap();
    assert!(reader.read(0).await.is_err());
    let first = reader.read(7).await.unwrap();
    assert_eq!(first, data[..7]);
    let mut actual = first;
    loop {
        let chunk = reader.read(usize::MAX).await.unwrap();
        assert!(chunk.len() <= 32 * 1024);
        if chunk.is_empty() {
            break;
        }
        actual.extend(chunk);
    }
    assert_eq!(actual, data);
    assert!(reader.read(1).await.unwrap().is_empty());
    reader.close().await.unwrap();
    let mut reader = remote.download(empty.to_str().unwrap()).await.unwrap();
    assert!(reader.read(100).await.unwrap().is_empty());
    reader.close().await.unwrap();
    session.close();
    closed(&session, no_prompts).await;
}

#[tokio::test]
async fn download_rejects_non_regular_objects_and_session_owns_reader_handles() {
    let Some(sshd) = Sshd::start() else { return };
    let source = sshd.root().join("download.bin");
    fs::write(&source, b"abc").unwrap();
    let link = sshd.root().join("link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&source, &link).unwrap();
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let remote = session.fs().unwrap();
    assert!(matches!(
        remote.download(sshd.root().to_str().unwrap()).await,
        Err(FsError::Unsupported(_))
    ));
    #[cfg(unix)]
    assert!(matches!(
        remote.download(link.to_str().unwrap()).await,
        Err(FsError::Unsupported(_))
    ));
    assert!(matches!(
        remote
            .download(sshd.root().join("missing").to_str().unwrap())
            .await,
        Err(FsError::NotFound { .. })
    ));
    let mut readers = Vec::new();
    for _ in 0..128 {
        readers.push(remote.download(source.to_str().unwrap()).await.unwrap());
    }
    assert!(remote.download(source.to_str().unwrap()).await.is_err());
    // Dropping alone releases an occupied slot and closes the server handle.
    drop(readers.pop());
    let mut replacement = timeout(EVENT_TIMEOUT, async {
        loop {
            if let Ok(reader) = remote.download(source.to_str().unwrap()).await {
                break reader;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(replacement.read(3).await.unwrap(), b"abc");
    session.close();
    closed(&session, no_prompts).await;
    assert!(matches!(
        replacement.read(3).await,
        Err(FsError::NotConnected)
    ));
    assert!(matches!(
        replacement.close().await,
        Err(FsError::NotConnected)
    ));
    drop(readers);
}

#[tokio::test]
async fn pipelined_upload_preserves_offsets_and_waits_before_publish() {
    let Some(sshd) = Sshd::start() else { return };
    let target = sshd.root().join("batch.bin");
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let remote = session.fs().unwrap();
    let mut writer = remote
        .upload(target.to_str().unwrap(), UploadMode::Create)
        .await
        .unwrap();
    assert!(writer.write_batch(vec![vec![0]; 9]).await.is_err());
    assert!(
        writer
            .write_batch(vec![vec![0; 32 * 1024 + 1]])
            .await
            .is_err()
    );
    let mut expected = Vec::new();
    for batch in 0..3 {
        let chunks: Vec<_> = (0..8)
            .map(|n| vec![batch * 8 + n; 32 * 1024 - usize::from(n)])
            .collect();
        expected.extend(chunks.concat());
        writer.write_batch(chunks).await.unwrap();
        assert!(!target.exists());
    }
    writer.write(b"last".to_vec()).await.unwrap();
    expected.extend(b"last");
    writer.finish().await.unwrap();
    assert_eq!(fs::read(target).unwrap(), expected);
    session.close();
    closed(&session, no_prompts).await;
}

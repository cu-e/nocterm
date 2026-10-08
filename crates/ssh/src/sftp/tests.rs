use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::time::{Duration, timeout};

async fn packet(peer: &mut DuplexStream) -> Vec<u8> {
    let len = peer.read_u32().await.unwrap() as usize;
    assert!(len <= MAX_CHUNK + 128);
    let mut bytes = vec![0; len];
    peer.read_exact(&mut bytes).await.unwrap();
    bytes
}
async fn send(peer: &mut DuplexStream, bytes: &[u8]) {
    peer.write_u32(bytes.len() as u32).await.unwrap();
    peer.write_all(bytes).await.unwrap();
}
async fn status(peer: &mut DuplexStream, id: u32, code: StatusCode) {
    let mut bytes = vec![101]; // SSH_FXP_STATUS
    bytes.extend(id.to_be_bytes());
    bytes.extend((code as u32).to_be_bytes());
    bytes.extend([0; 8]); // empty error message and language tag
    send(peer, &bytes).await;
}
fn request_id(packet: &[u8]) -> u32 {
    u32::from_be_bytes(packet[1..5].try_into().unwrap())
}

#[test]
fn hostile_directory_listing_is_bounded_and_names_cannot_escape_parent() {
    let mut budget = ListingBudget::default();
    assert!(budget.entry("ordinary:file\\name").is_ok());
    for name in ["", "../outside", "/outside", "a\0b"] {
        assert!(ListingBudget::default().entry(name).is_err());
    }
    assert!(
        ListingBudget::default()
            .entry(&"a".repeat(MAX_ENTRY_NAME + 1))
            .is_err()
    );
    let mut count = ListingBudget {
        entries: MAX_DIRECTORY_ENTRIES,
        ..Default::default()
    };
    assert!(count.entry("next").is_err());
    let mut bytes = ListingBudget {
        bytes: MAX_DIRECTORY_BYTES,
        ..Default::default()
    };
    assert!(bytes.entry("next").is_err());
    let mut pages = ListingBudget {
        pages: MAX_DIRECTORY_ENTRIES,
        ..Default::default()
    };
    assert!(pages.page().is_err());
}
#[tokio::test]
async fn upload_admission_never_opens_beyond_the_handle_limit() {
    let mut registry = HashMap::new();
    let upload = Arc::new(Mutex::new(Upload {
        temporary: "/tmp/staged".into(),
        destination: "/tmp/target".into(),
        handle: None,
        offset: 0,
        mode: UploadMode::Create,
        failed: false,
    }));
    for _ in 0..MAX_OPEN_UPLOADS {
        admit_upload(&mut registry, Uuid::new_v4(), upload.clone()).unwrap();
    }
    let excess = Uuid::new_v4();
    assert!(admit_upload(&mut registry, excess, upload.clone()).is_err());
    assert!(!registry.contains_key(&excess));
    let first = *registry.keys().next().unwrap();
    registry.remove(&first);
    admit_upload(&mut registry, excess, upload).unwrap();
    assert_eq!(registry.len(), MAX_OPEN_UPLOADS);
}

async fn directory_open(peer: &mut DuplexStream) {
    assert_eq!(packet(peer).await[0], 1);
    send(peer, &[2, 0, 0, 0, 3]).await;
    let open = packet(peer).await;
    assert_eq!(open[0], 11);
    let mut handle = vec![102];
    handle.extend(request_id(&open).to_be_bytes());
    handle.extend(4u32.to_be_bytes());
    handle.extend(b"list");
    send(peer, &handle).await;
}
async fn directory_names(peer: &mut DuplexStream, id: u32, names: &[String], mode: u32) {
    let mut bytes = vec![104];
    bytes.extend(id.to_be_bytes());
    bytes.extend((names.len() as u32).to_be_bytes());
    for name in names {
        bytes.extend((name.len() as u32).to_be_bytes());
        bytes.extend(name.as_bytes());
        bytes.extend(0u32.to_be_bytes());
        bytes.extend(4u32.to_be_bytes());
        bytes.extend(mode.to_be_bytes());
    }
    send(peer, &bytes).await;
}
#[tokio::test]
async fn rejected_directory_page_closes_its_remote_handle() {
    for name in ["../outside".into(), "x".repeat(MAX_ENTRY_NAME + 1)] {
        let (client, mut peer) = tokio::io::duplex(8192);
        let raw = RawSftpSession::new(client);
        let fake = tokio::spawn(async move {
            directory_open(&mut peer).await;
            let read = packet(&mut peer).await;
            assert_eq!(read[0], 12);
            directory_names(&mut peer, request_id(&read), &[name], 0o100600).await;
            let close = packet(&mut peer).await;
            assert_eq!(close[0], 4);
            status(&mut peer, request_id(&close), StatusCode::Ok).await;
        });
        timeout(Duration::from_secs(3), raw.init())
            .await
            .unwrap()
            .unwrap();
        let sftp = Sftp {
            raw,
            atomic_replace: false,
        };
        assert!(
            timeout(Duration::from_secs(3), read_dir(&sftp, "/root"))
                .await
                .unwrap()
                .is_err()
        );
        timeout(Duration::from_secs(3), fake)
            .await
            .unwrap()
            .unwrap();
    }
}
#[tokio::test]
async fn listing_pipelines_eight_link_stats_and_preserves_server_order() {
    let (client, mut peer) = tokio::io::duplex(8192);
    let raw = RawSftpSession::new(client);
    let names: Vec<_> = (0..16).map(|n| format!("link-{n}")).collect();
    let expected = names.clone();
    let fake = tokio::spawn(async move {
        directory_open(&mut peer).await;
        let read = packet(&mut peer).await;
        assert_eq!(read[0], 12);
        directory_names(&mut peer, request_id(&read), &names, 0o120777).await;
        for _ in 0..2 {
            let mut ids = Vec::new();
            // No response until the complete window arrives: sequential
            // enrichment would deadlock here and fail the timeout.
            for _ in 0..8 {
                let stat = packet(&mut peer).await;
                assert_eq!(stat[0], 17);
                ids.push(request_id(&stat));
            }
            for id in ids.into_iter().rev() {
                let mut attrs = vec![105];
                attrs.extend(id.to_be_bytes());
                attrs.extend(4u32.to_be_bytes());
                attrs.extend(0o040755u32.to_be_bytes());
                send(&mut peer, &attrs).await;
            }
        }
        let read = packet(&mut peer).await;
        assert_eq!(read[0], 12);
        status(&mut peer, request_id(&read), StatusCode::Eof).await;
        let close = packet(&mut peer).await;
        assert_eq!(close[0], 4);
        status(&mut peer, request_id(&close), StatusCode::Ok).await;
    });
    timeout(Duration::from_secs(3), raw.init())
        .await
        .unwrap()
        .unwrap();
    let sftp = Sftp {
        raw,
        atomic_replace: false,
    };
    let entries = timeout(Duration::from_secs(3), read_dir(&sftp, "/root"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(
        entries
            .iter()
            .all(|entry| entry.kind == EntryKind::Directory && entry.is_symlink)
    );
    timeout(Duration::from_secs(3), fake)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn download_closes_handle_when_fstat_rejects_object_changed_after_lstat() {
    let (client, mut peer) = tokio::io::duplex(1024);
    let raw = RawSftpSession::new(client);
    let fake = tokio::spawn(async move {
        assert_eq!(packet(&mut peer).await[0], 1);
        send(&mut peer, &[2, 0, 0, 0, 3]).await;
        let lstat = packet(&mut peer).await;
        assert_eq!(lstat[0], 7);
        let mut attrs = vec![105]; // ATTRS: permissions only
        attrs.extend(request_id(&lstat).to_be_bytes());
        attrs.extend(4u32.to_be_bytes());
        attrs.extend(0o100600u32.to_be_bytes());
        send(&mut peer, &attrs).await;
        let open = packet(&mut peer).await;
        assert_eq!(open[0], 3);
        let mut handle = vec![102];
        handle.extend(request_id(&open).to_be_bytes());
        handle.extend(6u32.to_be_bytes());
        handle.extend(b"handle");
        send(&mut peer, &handle).await;
        let fstat = packet(&mut peer).await;
        assert_eq!(fstat[0], 8);
        attrs[1..5].copy_from_slice(&request_id(&fstat).to_be_bytes());
        attrs[9..13].copy_from_slice(&0o040755u32.to_be_bytes());
        send(&mut peer, &attrs).await;
        let close = packet(&mut peer).await;
        assert_eq!(close[0], 4);
        status(&mut peer, request_id(&close), StatusCode::Ok).await;
    });
    timeout(Duration::from_secs(3), raw.init())
        .await
        .unwrap()
        .unwrap();
    let sftp = Sftp {
        raw,
        atomic_replace: false,
    };
    let downloads: Downloads = Arc::default();
    let (fs, _requests) = channel();
    assert!(matches!(
        timeout(
            Duration::from_secs(3),
            start_download(&sftp, &downloads, "/source".into(), fs)
        )
        .await
        .unwrap(),
        Err(FsError::Unsupported(_))
    ));
    assert!(downloads.lock().await.is_empty());
    timeout(Duration::from_secs(3), fake)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn batch_sends_full_window_drains_failure_and_never_publishes_partial_file() {
    let (client, mut peer) = tokio::io::duplex(64 * 1024);
    let raw = RawSftpSession::new(client);
    let (failure_sent, failure_received) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    let fake = tokio::spawn(async move {
        assert_eq!(packet(&mut peer).await[0], 1); // INIT
        send(&mut peer, &[2, 0, 0, 0, 3]).await; // VERSION 3
        let mut requests = Vec::new();
        let mut offset = 0;
        // Withhold every ACK until all eight WRITE packets arrive.
        for index in 0..MAX_WRITE_BATCH {
            let write = packet(&mut peer).await;
            assert_eq!(write[0], 6);
            let handle_len = u32::from_be_bytes(write[5..9].try_into().unwrap()) as usize;
            assert_eq!(&write[9..9 + handle_len], b"handle");
            let start = 9 + handle_len;
            assert_eq!(
                u64::from_be_bytes(write[start..start + 8].try_into().unwrap()),
                offset
            );
            let size =
                u32::from_be_bytes(write[start + 8..start + 12].try_into().unwrap()) as usize;
            assert_eq!(size, MAX_CHUNK - index);
            assert_eq!(&write[start + 12..], vec![index as u8; size]);
            offset += size as u64;
            requests.push(request_id(&write));
        }
        status(&mut peer, requests[0], StatusCode::Failure).await;
        failure_sent.send(()).unwrap();
        resume.await.unwrap();
        // Replies can be reordered. Every remaining reply must be drained.
        for id in requests.into_iter().skip(1).rev() {
            status(&mut peer, id, StatusCode::Ok).await;
        }
        // After rejected finish, cancellation closes and removes only the
        // temporary file. A RENAME would fail these packet assertions.
        let close = packet(&mut peer).await;
        assert_eq!(close[0], 4);
        status(&mut peer, request_id(&close), StatusCode::Ok).await;
        let remove = packet(&mut peer).await;
        assert_eq!(remove[0], 13);
        assert_eq!(&remove[9..], b"/tmp/.partial");
        status(&mut peer, request_id(&remove), StatusCode::Ok).await;
    });
    timeout(Duration::from_secs(3), raw.init())
        .await
        .unwrap()
        .unwrap();
    let sftp = Arc::new(Sftp {
        raw,
        atomic_replace: false,
    });
    let uploads: Uploads = Arc::default();
    let id = Uuid::new_v4();
    uploads.lock().await.insert(
        id,
        Arc::new(Mutex::new(Upload {
            temporary: "/tmp/.partial".into(),
            destination: "/tmp/target".into(),
            handle: Some("handle".into()),
            offset: 0,
            mode: UploadMode::Create,
            failed: false,
        })),
    );
    let write = tokio::spawn({
        let sftp = sftp.clone();
        let uploads = uploads.clone();
        async move {
            write_batch(
                &sftp,
                &uploads,
                id,
                (0..MAX_WRITE_BATCH)
                    .map(|n| vec![n as u8; MAX_CHUNK - n])
                    .collect(),
            )
            .await
        }
    });
    timeout(Duration::from_secs(3), failure_received)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !write.is_finished(),
        "an early failed ACK must not abandon the remaining requests"
    );
    let publish = tokio::spawn({
        let sftp = sftp.clone();
        let uploads = uploads.clone();
        async move { finish(&sftp, &uploads, id).await }
    });
    assert!(!publish.is_finished());
    release.send(()).unwrap();
    assert!(
        timeout(Duration::from_secs(3), write)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(
        timeout(Duration::from_secs(3), publish)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let upload = get_upload(&uploads, id).await.unwrap();
    assert!(upload.lock().await.failed);
    assert_eq!(upload.lock().await.offset, 0);
    assert!(
        write_batch(&sftp, &uploads, id, vec![b"retry".to_vec()])
            .await
            .is_err()
    );
    timeout(Duration::from_secs(3), cancel(&sftp, &uploads, id))
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(3), fake)
        .await
        .unwrap()
        .unwrap();
    assert!(uploads.lock().await.is_empty());
}

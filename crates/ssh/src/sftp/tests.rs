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
            timeout(Duration::from_secs(3), listing(&sftp, "/root"))
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
    let entries = timeout(Duration::from_secs(3), listing(&sftp, "/root"))
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
    let (fs, _requests) = channel(Default::default());
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
#[expect(clippy::too_many_lines, reason = "predates the limit")]
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

#[tokio::test]
async fn uncertain_rename_reply_cannot_authorize_retry_even_when_both_paths_exist() {
    let (client, mut peer) = tokio::io::duplex(4096);
    let raw = RawSftpSession::new(client);
    let server = tokio::spawn(async move {
        assert_eq!(packet(&mut peer).await[0], 1);
        send(&mut peer, &[2, 0, 0, 0, 3]).await;
        let rename = packet(&mut peer).await;
        assert_eq!(rename[0], 18);
        let mut unexpected = vec![105]; // ATTRS cannot acknowledge RENAME.
        unexpected.extend(request_id(&rename).to_be_bytes());
        unexpected.extend(0u32.to_be_bytes());
        send(&mut peer, &unexpected).await;
        loop {
            let request = packet(&mut peer).await;
            match request[0] {
                7 => {
                    // Fresh observations of both names are not a definitive
                    // rejection of the earlier rename request.
                    let mut attrs = vec![105];
                    attrs.extend(request_id(&request).to_be_bytes());
                    attrs.extend(4u32.to_be_bytes());
                    attrs.extend(0o100600u32.to_be_bytes());
                    send(&mut peer, &attrs).await;
                }
                13 => {
                    assert_eq!(&request[9..], b"/tmp/.partial");
                    status(&mut peer, request_id(&request), StatusCode::Ok).await;
                    break;
                }
                other => panic!("uncertain publication issued another operation: {other}"),
            }
        }
    });
    timeout(Duration::from_secs(3), raw.init())
        .await
        .unwrap()
        .unwrap();
    let sftp = Sftp {
        raw,
        atomic_replace: false,
    };
    let uploads: Uploads = Arc::default();
    let id = Uuid::new_v4();
    uploads.lock().await.insert(
        id,
        Arc::new(Mutex::new(Upload {
            temporary: "/tmp/.partial".into(),
            destination: "/tmp/target".into(),
            handle: None,
            offset: 4,
            mode: UploadMode::Create,
            failed: false,
        })),
    );
    let result = timeout(Duration::from_secs(3), finish(&sftp, &uploads, id))
        .await
        .unwrap();
    assert!(
        matches!(result, Err(FsError::Other(_))),
        "an unexpected rename acknowledgement must not become a retryable collision: {result:?}"
    );
    assert!(
        publish_as(&sftp, &uploads, id, "retry".into())
            .await
            .is_err()
    );
    timeout(Duration::from_secs(3), cancel(&sftp, &uploads, id))
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}

/// Reads one SSH_FXP_READ request: (request id, offset, length).
async fn read_request(peer: &mut DuplexStream) -> (u32, u64, u32) {
    let read = packet(peer).await;
    assert_eq!(read[0], 5);
    let handle_len = u32::from_be_bytes(read[5..9].try_into().unwrap()) as usize;
    assert_eq!(&read[9..9 + handle_len], b"handle");
    let start = 9 + handle_len;
    (
        request_id(&read),
        u64::from_be_bytes(read[start..start + 8].try_into().unwrap()),
        u32::from_be_bytes(read[start + 8..start + 12].try_into().unwrap()),
    )
}
async fn data(peer: &mut DuplexStream, id: u32, bytes: &[u8]) {
    let mut reply = vec![103]; // SSH_FXP_DATA
    reply.extend(id.to_be_bytes());
    reply.extend((bytes.len() as u32).to_be_bytes());
    reply.extend(bytes);
    send(peer, &reply).await;
}
async fn download_session(client: DuplexStream, offset: u64) -> (Arc<Sftp>, Downloads, Uuid) {
    let raw = RawSftpSession::new(client);
    let sftp = Arc::new(Sftp {
        raw,
        atomic_replace: false,
    });
    let downloads: Downloads = Arc::default();
    let id = Uuid::new_v4();
    downloads.lock().await.insert(
        id,
        Arc::new(Mutex::new(Download {
            source: "/source".into(),
            handle: Some("handle".into()),
            offset,
            eof: false,
        })),
    );
    (sftp, downloads, id)
}
async fn handshake(peer: &mut DuplexStream) {
    assert_eq!(packet(peer).await[0], 1);
    send(peer, &[2, 0, 0, 0, 3]).await;
}

#[tokio::test]
async fn read_window_sends_every_read_before_awaiting_a_reply_and_keeps_order() {
    let (client, mut peer) = tokio::io::duplex(64 * 1024);
    let fake = tokio::spawn(async move {
        handshake(&mut peer).await;
        let mut requests = Vec::new();
        // No reply until the whole window arrived: sequential reads deadlock here.
        for index in 0..4u64 {
            let (id, offset, length) = read_request(&mut peer).await;
            assert_eq!((offset, length), (1000 + index * 512, 512));
            requests.push((id, index as u8));
        }
        for (id, index) in requests.into_iter().rev() {
            data(&mut peer, id, &[index; 512]).await;
        }
    });
    let (sftp, downloads, id) = download_session(client, 1000).await;
    timeout(Duration::from_secs(3), sftp.raw.init())
        .await
        .unwrap()
        .unwrap();
    let chunks = timeout(
        Duration::from_secs(3),
        read_batch(&sftp, &downloads, id, 512, 4),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        chunks,
        (0..4u8).map(|index| vec![index; 512]).collect::<Vec<_>>()
    );
    let download = downloads.lock().await.get(&id).cloned().unwrap();
    assert_eq!(download.lock().await.offset, 1000 + 4 * 512);
    timeout(Duration::from_secs(3), fake)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn short_read_keeps_the_prefix_and_the_next_batch_continues_from_its_end() {
    let (client, mut peer) = tokio::io::duplex(64 * 1024);
    let fake = tokio::spawn(async move {
        handshake(&mut peer).await;
        let mut ids = Vec::new();
        for _ in 0..4 {
            ids.push(read_request(&mut peer).await.0);
        }
        data(&mut peer, ids[0], &[1; 512]).await;
        data(&mut peer, ids[1], &[2; 100]).await;
        // Data at the offsets after a short reply must never be returned.
        data(&mut peer, ids[2], &[9; 512]).await;
        data(&mut peer, ids[3], &[9; 512]).await;
        let (id, offset, _) = read_request(&mut peer).await;
        assert_eq!(offset, 612);
        data(&mut peer, id, &[3; 512]).await;
    });
    let (sftp, downloads, id) = download_session(client, 0).await;
    timeout(Duration::from_secs(3), sftp.raw.init())
        .await
        .unwrap()
        .unwrap();
    let chunks = timeout(
        Duration::from_secs(3),
        read_batch(&sftp, &downloads, id, 512, 4),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(chunks, vec![vec![1; 512], vec![2; 100]]);
    let next = timeout(
        Duration::from_secs(3),
        read_batch(&sftp, &downloads, id, 512, 1),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(next, vec![vec![3; 512]]);
    timeout(Duration::from_secs(3), fake)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn eof_ends_the_prefix_and_later_batches_are_empty_without_requests() {
    let (client, mut peer) = tokio::io::duplex(64 * 1024);
    let fake = tokio::spawn(async move {
        handshake(&mut peer).await;
        let mut ids = Vec::new();
        for _ in 0..3 {
            ids.push(read_request(&mut peer).await.0);
        }
        data(&mut peer, ids[0], &[1; 512]).await;
        status(&mut peer, ids[1], StatusCode::Eof).await;
        status(&mut peer, ids[2], StatusCode::Eof).await;
        // Any further request would be unanswered and time the caller out.
    });
    let (sftp, downloads, id) = download_session(client, 0).await;
    timeout(Duration::from_secs(3), sftp.raw.init())
        .await
        .unwrap()
        .unwrap();
    let chunks = timeout(
        Duration::from_secs(3),
        read_batch(&sftp, &downloads, id, 512, 3),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(chunks, vec![vec![1; 512]]);
    let end = timeout(
        Duration::from_secs(3),
        read_batch(&sftp, &downloads, id, 512, 3),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(end.is_empty());
    timeout(Duration::from_secs(3), fake)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn failed_first_read_is_an_error_but_a_later_failure_keeps_the_prefix() {
    let (client, mut peer) = tokio::io::duplex(64 * 1024);
    let fake = tokio::spawn(async move {
        handshake(&mut peer).await;
        let mut ids = Vec::new();
        for _ in 0..2 {
            ids.push(read_request(&mut peer).await.0);
        }
        status(&mut peer, ids[0], StatusCode::PermissionDenied).await;
        data(&mut peer, ids[1], &[1; 512]).await;
        let mut ids = Vec::new();
        for _ in 0..2 {
            ids.push(read_request(&mut peer).await.0);
        }
        data(&mut peer, ids[0], &[2; 512]).await;
        status(&mut peer, ids[1], StatusCode::Failure).await;
    });
    let (sftp, downloads, id) = download_session(client, 0).await;
    timeout(Duration::from_secs(3), sftp.raw.init())
        .await
        .unwrap()
        .unwrap();
    let failed = timeout(
        Duration::from_secs(3),
        read_batch(&sftp, &downloads, id, 512, 2),
    )
    .await
    .unwrap();
    assert!(matches!(failed, Err(FsError::PermissionDenied { .. })));
    let partial = timeout(
        Duration::from_secs(3),
        read_batch(&sftp, &downloads, id, 512, 2),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(partial, vec![vec![2; 512]]);
    timeout(Duration::from_secs(3), fake)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn read_batch_refuses_oversized_windows_without_touching_the_connection() {
    let (client, _peer) = tokio::io::duplex(1024);
    let (sftp, downloads, id) = download_session(client, 0).await;
    for (max_bytes, count) in [(512, 0), (512, MAX_READ_BATCH + 1), (0, 1)] {
        assert!(matches!(
            read_batch(&sftp, &downloads, id, max_bytes, count).await,
            Err(FsError::Other(_))
        ));
    }
}

async fn listing(sftp: &Sftp, path: &str) -> Result<Vec<DirEntry>, FsError> {
    let (mut reply, _result) = oneshot::channel();
    read_dir(sftp, path, &mut reply).await
}

#[tokio::test]
async fn cancelling_a_listing_closes_its_handle_without_waiting_for_a_page() {
    let (client, mut peer) = tokio::io::duplex(8192);
    let raw = RawSftpSession::new(client);
    let (reading, started) = tokio::sync::oneshot::channel();
    let fake = tokio::spawn(async move {
        directory_open(&mut peer).await;
        let read = packet(&mut peer).await;
        assert_eq!(read[0], 12);
        reading.send(()).unwrap();
        let close = packet(&mut peer).await;
        assert_eq!(close[0], 4);
        status(&mut peer, request_id(&close), StatusCode::Ok).await;
    });
    raw.init().await.unwrap();
    let sftp = Sftp {
        raw,
        atomic_replace: false,
    };
    let (mut reply, result) = oneshot::channel();
    let (listing, ()) = tokio::join!(read_dir(&sftp, "/root", &mut reply), async {
        started.await.unwrap();
        drop(result);
    });
    assert!(listing.is_err());
    timeout(Duration::from_secs(3), fake)
        .await
        .unwrap()
        .unwrap();
}

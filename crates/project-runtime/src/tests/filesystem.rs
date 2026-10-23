use super::*;
use futures::FutureExt as _;
use nocterm_session::{
    Auth, ConnectRequest, ExecExit, FsError, ProxyConfig, PtySize, RemoteFs, ShellLaunch, Target,
    fs::UploadMode,
    testing::{InMemoryFs, ScriptedTransport},
};
use std::time::Duration;

fn open(exec: &FakeExec, engine: Engine) -> Arc<dyn RemoteFs> {
    let base = ScriptedTransport::default();
    base.queue(
        Some(Arc::new(InMemoryFs::default())),
        Some(Arc::new(exec.clone())),
    );
    let location = ContainerLocation::new(
        HostLocator::Local,
        engine,
        id(),
        RootPath::parse("/workspace vanished").unwrap(),
        ContainerOrigin::Attached,
    )
    .unwrap();
    let session = container_sessions()
        .wrap(Arc::new(base), location)
        .open(ConnectRequest {
            target: Target::new("user", "remote-engine", 22),
            auth: Auth::Auto,
            jumps: vec![],
            term: "xterm".into(),
            size: PtySize::default(),
            proxy: ProxyConfig::default(),
            launch: ShellLaunch::default(),
            connect_timeout: Duration::from_secs(5),
            keepalive_interval: None,
        });
    session.fs().unwrap()
}
fn response(exec: &FakeExec, engine: Engine, bytes: impl Into<Vec<u8>>, status: u32) {
    let sink = exec.queue_stream(format!("{} exec", engine.command().program));
    assert_eq!(sink.send(bytes.into()).now_or_never(), Some(true));
    sink.finish(ExecExit {
        status: Some(status),
        stderr: String::new(),
    });
}
fn ready(exec: &FakeExec, engine: Engine) {
    response(exec, engine, b"nocterm-fs-v1".to_vec(), 0);
}
fn upload_ready(exec: &FakeExec) {
    ready(exec, Engine::Docker);
    response(exec, Engine::Docker, b"1:2\n".to_vec(), 0);
    response(exec, Engine::Docker, b"1:3\x001:4\x00".to_vec(), 0);
}

#[test]
fn container_filesystem_routes_both_engines_and_preserves_nul_delimited_names() {
    for engine in Engine::ALL {
        let exec = FakeExec::default();
        let fs = open(&exec, engine);
        assert!(!fs.capabilities().metadata);
        ready(&exec, engine);
        response(&exec, engine, b"/home/user".to_vec(), 0);
        assert_eq!(block_on(fs.home()).unwrap(), "/home/user");
        assert!(fs.capabilities().metadata);
        response(
            &exec,
            engine,
            b"quote'\n$(literal)\x001:2:81a4:3:644:1:2:5\x00".to_vec(),
            0,
        );
        let entries = block_on(fs.read_dir("/dir ' $(no)\n")).unwrap();
        assert_eq!(entries[0].name, "quote'\n$(literal)");
        assert_eq!(entries[0].size, Some(3));
        for request in exec.requests() {
            assert_eq!(request.program, engine.command().program);
            assert_eq!(
                &request.args[..5],
                &["exec", "--interactive", "--workdir", "/", &id()]
            );
            assert!(!request.args.iter().any(|arg| arg == "--user"));
        }
        let requests = exec.requests();
        let listing = requests.last().unwrap();
        assert_eq!(listing.args.last().unwrap(), "/dir ' $(no)\n");
        assert!(!listing.args[7].contains("/dir ' $(no)"));
    }
}
#[test]
fn invalid_paths_never_execute_and_probe_failures_do_not_advertise_mutations() {
    let exec = FakeExec::default();
    let fs = open(&exec, Engine::Docker);
    for path in [
        "relative",
        "/parent/../file",
        "/parent/./file",
        "/nul\0file",
    ] {
        assert!(block_on(fs.read_dir(path)).is_err());
    }
    assert!(exec.requests().is_empty());
    response(&exec, Engine::Docker, vec![], 76);
    assert!(matches!(block_on(fs.home()), Err(FsError::Unsupported(_))));
    assert!(!fs.capabilities().remove);
    assert!(matches!(block_on(fs.home()), Err(FsError::Unsupported(_))));
    assert_eq!(exec.requests().len(), 1);
}
#[test]
fn typed_metadata_missing_permission_and_malformed_names_are_preserved() {
    let exec = FakeExec::default();
    let fs = open(&exec, Engine::Docker);
    ready(&exec, Engine::Docker);
    response(&exec, Engine::Docker, vec![], 72);
    assert_eq!(block_on(fs.stat("/missing")).unwrap(), None);
    response(&exec, Engine::Docker, vec![], 71);
    assert!(matches!(
        block_on(fs.metadata("/private")),
        Err(FsError::PermissionDenied { .. })
    ));
    response(&exec, Engine::Docker, vec![255, 0], 0);
    assert!(block_on(fs.read_dir("/")).is_err());
}
#[test]
fn upload_collision_seals_writes_and_allows_same_parent_publication_retry() {
    let exec = FakeExec::default();
    let fs = open(&exec, Engine::Docker);
    upload_ready(&exec);
    let mut upload = block_on(fs.upload("/parent/file", UploadMode::Create)).unwrap();
    response(&exec, Engine::Docker, b"3".to_vec(), 0);
    block_on(upload.write(vec![0, 1, 255])).unwrap();
    response(&exec, Engine::Docker, vec![], 73);
    assert!(matches!(
        block_on(upload.publish_as("collision".into())),
        Err(FsError::AlreadyExists { .. })
    ));
    assert!(block_on(upload.write(vec![2])).is_err());
    response(&exec, Engine::Docker, b"published".to_vec(), 0);
    assert_eq!(
        block_on(upload.publish_as("new name".into())).unwrap(),
        "/parent/new name"
    );
    let requests = exec.requests();
    assert_eq!(requests[3].stdin, Some(vec![0, 1, 255]));
    assert_eq!(requests[3].args.last().unwrap(), "3");
    assert!(requests[4].args.iter().any(|arg| arg == "collision"));
    assert!(requests[5].args.iter().any(|arg| arg == "new name"));
}
#[test]
fn oversized_chunks_poison_publication_and_drop_cleans_only_recorded_stage() {
    let exec = FakeExec::default();
    let fs = open(&exec, Engine::Docker);
    upload_ready(&exec);
    let mut upload = block_on(fs.upload("/parent/file", UploadMode::Replace)).unwrap();
    assert!(block_on(upload.write(vec![0; 32769])).is_err());
    assert!(block_on(upload.publish_as("new".into())).is_err());
    response(&exec, Engine::Docker, vec![], 0);
    response(&exec, Engine::Docker, b"/root".to_vec(), 0);
    drop(upload);
    block_on(fs.home()).unwrap();
    let requests = exec.requests();
    assert_eq!(requests.len(), 5);
    let cleanup = &requests[3];
    assert!(cleanup.args.iter().any(|arg| arg == "1:3"));
    assert!(cleanup.args.iter().any(|arg| arg == "1:4"));
    assert!(cleanup.args[7].contains("rmdir"));
}
#[test]
fn changed_parent_or_stage_refuses_append_and_cannot_publish() {
    let exec = FakeExec::default();
    let fs = open(&exec, Engine::Docker);
    upload_ready(&exec);
    let mut upload = block_on(fs.upload("/parent/file", UploadMode::Create)).unwrap();
    response(&exec, Engine::Docker, vec![], 75);
    assert!(block_on(upload.write(vec![7])).is_err());
    let count = exec.requests().len();
    assert!(block_on(upload.publish_as("other".into())).is_err());
    assert_eq!(exec.requests().len(), count);
    response(&exec, Engine::Docker, vec![], 75);
    assert!(block_on(upload.cancel()).is_err());
}
#[test]
fn download_streams_beyond_exec_backlog_and_respects_requested_read_size() {
    let exec = FakeExec::default();
    let fs = open(&exec, Engine::Docker);
    ready(&exec, Engine::Docker);
    let sink = exec.queue_stream("docker exec");
    let data: Vec<_> = (0..1024 * 1024 + 33)
        .map(|index| (index % 251) as u8)
        .collect();
    let expected = data.clone();
    let producer = std::thread::spawn(move || {
        block_on(async move {
            assert!(sink.send(format!("{}\0", data.len()).into_bytes()).await);
            for chunk in data.chunks(8192) {
                assert!(sink.send(chunk.to_vec()).await);
            }
            sink.finish(ExecExit {
                status: Some(0),
                stderr: String::new(),
            });
        })
    });
    let mut download = block_on(fs.download("/large")).unwrap();
    let mut actual = Vec::new();
    loop {
        let chunk = block_on(download.read(777)).unwrap();
        assert!(chunk.len() <= 777);
        if chunk.is_empty() {
            break;
        }
        actual.extend(chunk);
    }
    block_on(download.close()).unwrap();
    producer.join().unwrap();
    assert_eq!(actual, expected);
}
#[test]
fn download_refuses_special_files_and_detects_truncation() {
    let exec = FakeExec::default();
    let fs = open(&exec, Engine::Docker);
    ready(&exec, Engine::Docker);
    response(&exec, Engine::Docker, vec![], 76);
    assert!(matches!(
        block_on(fs.download("/pipe")),
        Err(FsError::Unsupported(_))
    ));
    response(&exec, Engine::Docker, b"5\0ab".to_vec(), 0);
    let mut download = block_on(fs.download("/truncated")).unwrap();
    assert_eq!(block_on(download.read(8)).unwrap(), b"ab");
    assert!(block_on(download.read(8)).is_err());
}

async fn observed(exec: &FakeExec, count: usize) {
    for _ in 0..1000 {
        if exec.requests().len() >= count {
            return;
        }
        async_io::Timer::after(Duration::from_millis(1)).await;
    }
    panic!("filesystem command was not observed");
}
#[test]
fn abandoned_append_acknowledgement_prevents_publication_and_releases_process() {
    block_on(async {
        let exec = FakeExec::default();
        let fs = open(&exec, Engine::Docker);
        upload_ready(&exec);
        let mut upload = fs.upload("/parent/file", UploadMode::Create).await.unwrap();
        let sink = exec.queue_stream("docker exec");
        let write = upload.write(vec![1]);
        let observed = observed(&exec, 4);
        futures::pin_mut!(observed);
        match futures::future::select(write, observed).await {
            futures::future::Either::Right(((), pending)) => drop(pending),
            _ => panic!("write unexpectedly acknowledged"),
        }
        assert!(upload.publish_as("other".into()).await.is_err());
        assert_eq!(exec.requests().len(), 4);
        assert!(sink.is_closed());
        response(&exec, Engine::Docker, vec![], 0);
        upload.cancel().await.unwrap();
    });
}
#[test]
fn abandoned_download_read_cancels_held_output_without_starving_other_operations() {
    block_on(async {
        let exec = FakeExec::default();
        let fs = open(&exec, Engine::Docker);
        ready(&exec, Engine::Docker);
        let sink = exec.queue_stream("docker exec");
        assert!(sink.send(b"2\0a".to_vec()).await);
        let mut download = fs.download("/held").await.unwrap();
        assert_eq!(download.read(1).await.unwrap(), b"a");
        let read = download.read(1);
        let pending = async_io::Timer::after(Duration::from_millis(20));
        futures::pin_mut!(pending);
        match futures::future::select(read, pending).await {
            futures::future::Either::Right((_, pending)) => drop(pending),
            _ => panic!("read unexpectedly completed"),
        }
        response(&exec, Engine::Docker, b"/root".to_vec(), 0);
        assert_eq!(fs.home().await.unwrap(), "/root");
        assert!(sink.is_closed());
        download.close().await.unwrap();
    });
}

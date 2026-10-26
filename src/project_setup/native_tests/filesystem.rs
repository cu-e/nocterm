//! Public filesystem contracts against disposable containers, never host paths.
use super::{environment::native_shell, *};
use nocterm_session::{EntryKind, FsError, RemoteFs, fs::UploadMode};
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires Docker and cached Ubuntu; exercises real container file transfers"]
fn a_real_ubuntu_workspace_exposes_its_own_streaming_filesystem() {
    container_filesystem("docker.io/library/ubuntu:24.04");
}

#[test]
#[ignore = "requires Docker and cached Alpine; exercises BusyBox container file transfers"]
fn a_real_alpine_workspace_exposes_its_own_streaming_filesystem() {
    container_filesystem("docker.io/library/alpine:3.22");
}

fn container_filesystem(image: &str) {
    let (runtime, exec, owned) = prepare_native(WorkspaceImage::Custom(image.into()), vec![]);
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (session, _driver) = container_session(exec.clone(), &owned);
        let fs = session.fs().expect("container has scoped file access");
        block_on(exercise_filesystem(&fs));
        reject_special_files(&exec, &owned, &fs);
        cleanup_abandoned_upload(&fs);
        native_shell(&exec, &owned, "mv /workspace /workspace-moved");
        assert!(
            block_on(fs.read_dir("/"))
                .unwrap()
                .iter()
                .any(|entry| entry.name == "workspace-moved")
        );
    }));
    let removed = block_on(runtime.discard_owned(exec, owned));
    checked.unwrap();
    removed.unwrap();
}

#[test]
#[ignore = "requires Docker and cached Ubuntu; exercises real UID 1001 file access"]
fn a_real_container_filesystem_obeys_its_engine_account_permissions() {
    let (runtime, exec, owned) = prepare_native(
        WorkspaceImage::Custom("docker.io/library/ubuntu:24.04".into()),
        vec![],
    );
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        native_shell(
            &exec,
            &owned,
            "set -eu; mkdir -p /home/nocterm-fs-test /workspace/fs-user; chown 1001:1001 /home/nocterm-fs-test /workspace/fs-user; printf 'private' > /workspace/root-only; chmod 600 /workspace/root-only",
        );
        let account = Arc::new(EngineAccount {
            base: exec.clone(),
            container: owned.location().container().into(),
        });
        let (session, _driver) = container_session(account, &owned);
        let fs = session.fs().unwrap();
        block_on(async {
            assert_eq!(fs.home().await.unwrap(), "/home/nocterm-fs-test");
            write_file(
                &fs,
                "/workspace/fs-user/owned.bin",
                UploadMode::Create,
                b"uid 1001",
            )
            .await;
            assert_eq!(
                fs.metadata("/workspace/fs-user/owned.bin")
                    .await
                    .unwrap()
                    .uid,
                Some(1001)
            );
            assert_eq!(
                read_file(&fs, "/workspace/fs-user/owned.bin").await,
                b"uid 1001"
            );
            assert!(fs.download("/workspace/root-only").await.is_err());
            assert!(
                fs.upload("/workspace/refused", UploadMode::Create)
                    .await
                    .is_err()
            );
            fs.remove_file("/workspace/fs-user/owned.bin")
                .await
                .unwrap();
        });
    }));
    let removed = block_on(runtime.discard_owned(exec, owned));
    checked.unwrap();
    removed.unwrap();
}

/// A real engine source constraining only the exact disposable resource's account.
/// This exercises normal non-root permissions without building another image.
struct EngineAccount {
    base: Arc<nocterm_local::LocalExec>,
    container: String,
}
impl nocterm_session::HostExec for EngineAccount {
    fn exec(
        &self,
        mut request: ExecRequest,
    ) -> nocterm_session::ExecFuture<nocterm_session::ExecOutput> {
        assert_eq!(request.program, "docker");
        assert_eq!(request.args.first().map(String::as_str), Some("exec"));
        assert!(request.args.contains(&self.container));
        request.args.splice(
            1..1,
            ["--user", "1001:1001", "--env", "HOME=/home/nocterm-fs-test"].map(String::from),
        );
        self.base.exec(request)
    }
}

async fn exercise_filesystem(fs: &Arc<dyn RemoteFs>) {
    assert_eq!(fs.home().await.unwrap(), "/root");
    assert!(fs.capabilities().metadata);
    fs.create_dir("/workspace/fs-contract").await.unwrap();
    assert_eq!(
        fs.metadata("/workspace/fs-contract").await.unwrap().kind,
        EntryKind::Directory
    );
    let path = "/workspace/fs-contract/a 'quoted' $(literal)\nname.bin";
    binary_roundtrip_and_edit(fs, path).await;
    collision_retains_sealed_stage(fs).await;
    rename_and_remove_entries(fs, path).await;
}

async fn binary_roundtrip_and_edit(fs: &Arc<dyn RemoteFs>, path: &str) {
    let bytes: Vec<u8> = (0..1_048_729).map(|index| (index % 251) as u8).collect();
    write_file(fs, path, UploadMode::Create, &bytes).await;
    let entries = fs.read_dir("/workspace/fs-contract").await.unwrap();
    assert!(
        entries
            .iter()
            .any(|entry| entry.name == "a 'quoted' $(literal)\nname.bin")
    );
    let metadata = fs.metadata(path).await.unwrap();
    assert_eq!(metadata.size, Some(bytes.len() as u64));
    assert!(!metadata.is_symlink);
    fs.set_permissions(path, 0o600).await.unwrap();
    assert_eq!(fs.metadata(path).await.unwrap().permissions, Some(0o600));
    assert_eq!(read_file(fs, path).await, bytes);
    write_file(fs, path, UploadMode::Replace, b"edited\0contents").await;
    assert_eq!(read_file(fs, path).await, b"edited\0contents");
}

async fn rename_and_remove_entries(fs: &Arc<dyn RemoteFs>, path: &str) {
    fs.rename(path, "/workspace/fs-contract/renamed.bin")
        .await
        .unwrap();
    assert!(matches!(
        fs.rename(
            "/workspace/fs-contract/renamed.bin",
            "/workspace/fs-contract/collision.bin"
        )
        .await,
        Err(FsError::AlreadyExists { .. })
    ));
    for name in ["renamed.bin", "collision.bin", "retry.bin"] {
        fs.remove_file(&format!("/workspace/fs-contract/{name}"))
            .await
            .unwrap();
    }
    fs.create_dir("/workspace/fs-contract/empty").await.unwrap();
    fs.remove_dir("/workspace/fs-contract/empty").await.unwrap();
    assert!(
        fs.stat("/workspace/fs-contract/empty")
            .await
            .unwrap()
            .is_none()
    );
}

async fn write_file(fs: &Arc<dyn RemoteFs>, path: &str, mode: UploadMode, bytes: &[u8]) {
    let mut upload = fs.upload(path, mode).await.unwrap();
    for batch in bytes.chunks(8 * 32 * 1024) {
        upload
            .write_batch(batch.chunks(32 * 1024).map(<[u8]>::to_vec).collect())
            .await
            .unwrap();
    }
    assert_eq!(upload.finish().await.unwrap(), path);
}

async fn read_file(fs: &Arc<dyn RemoteFs>, path: &str) -> Vec<u8> {
    let mut download = fs.download(path).await.unwrap();
    let mut bytes = Vec::new();
    loop {
        let chunk = download.read(8192).await.unwrap();
        assert!(chunk.len() <= 8192);
        if chunk.is_empty() {
            break;
        }
        bytes.extend(chunk);
    }
    download.close().await.unwrap();
    bytes
}

async fn collision_retains_sealed_stage(fs: &Arc<dyn RemoteFs>) {
    let path = "/workspace/fs-contract/collision.bin";
    let mut late = fs.upload(path, UploadMode::Create).await.unwrap();
    late.write(b"retained stage".to_vec()).await.unwrap();
    write_file(fs, path, UploadMode::Create, b"external winner").await;
    assert!(matches!(
        late.publish_as("collision.bin".into()).await,
        Err(FsError::AlreadyExists { .. })
    ));
    assert!(late.write(b"must not append".to_vec()).await.is_err());
    assert_eq!(
        late.publish_as("retry.bin".into()).await.unwrap(),
        "/workspace/fs-contract/retry.bin"
    );
    drop(late);
    assert_eq!(read_file(fs, path).await, b"external winner");
    assert_eq!(
        read_file(fs, "/workspace/fs-contract/retry.bin").await,
        b"retained stage"
    );
}

fn reject_special_files(
    exec: &Arc<nocterm_local::LocalExec>,
    owned: &OwnedContainer,
    fs: &Arc<dyn RemoteFs>,
) {
    native_shell(
        exec,
        owned,
        "set -eu; printf 'target' > /workspace/fs-contract/target; ln -s target /workspace/fs-contract/link; mkfifo /workspace/fs-contract/pipe",
    );
    block_on(async {
        assert!(
            fs.metadata("/workspace/fs-contract/link")
                .await
                .unwrap()
                .is_symlink
        );
        for name in ["link", "pipe"] {
            let path = format!("/workspace/fs-contract/{name}");
            let started = Instant::now();
            assert!(
                fs.download(&path).await.is_err(),
                "opened special file {name}"
            );
            assert!(started.elapsed() < Duration::from_secs(5));
            assert!(fs.upload(&path, UploadMode::Replace).await.is_err());
        }
        fs.remove_file("/workspace/fs-contract/link").await.unwrap();
        assert_eq!(
            read_file(fs, "/workspace/fs-contract/target").await,
            b"target"
        );
    });
}

fn cleanup_abandoned_upload(fs: &Arc<dyn RemoteFs>) {
    block_on(async {
        let mut before = fs.read_dir("/workspace/fs-contract").await.unwrap();
        before.sort_by(|a, b| a.name.cmp(&b.name));
        let mut upload = fs
            .upload("/workspace/fs-contract/abandoned", UploadMode::Create)
            .await
            .unwrap();
        upload.write(b"never publish".to_vec()).await.unwrap();
        drop(upload);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut after = fs.read_dir("/workspace/fs-contract").await.unwrap();
            after.sort_by(|a, b| a.name.cmp(&b.name));
            if before == after {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "abandoned upload stage survived: {after:?}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    });
}

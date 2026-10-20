use nocterm_session::{RemoteFs, Target, local_fs::LocalFs, testing::InMemoryFs};
use nocterm_transfer_runtime::{TransferRuntime, TransferRuntimeDeps};
use nocterm_transfers::{
    CollisionPolicy, DownloadRequest, Progress, TransferId, TransferLimits, TransferService,
    UploadRequest,
};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) struct Fixture {
    pub(super) runtime: Arc<TransferRuntime>,
    pub(super) local: Arc<InMemoryFs>,
    pub(super) remote: Arc<InMemoryFs>,
}
impl Fixture {
    pub(super) fn new() -> Self {
        Self::with_limits(TransferLimits::default())
    }
    pub(super) fn with_limits(limits: TransferLimits) -> Self {
        let local = Arc::new(InMemoryFs::default());
        local.seed_native_directory(&native("input")).unwrap();
        local.seed_native_directory(&native("output")).unwrap();
        let remote = Arc::new(InMemoryFs::default());
        futures::executor::block_on(remote.create_dir("/dest")).unwrap();
        let runtime =
            TransferRuntime::start(TransferRuntimeDeps::new(local.clone(), limits)).unwrap();
        Self {
            runtime,
            local,
            remote,
        }
    }
    pub(super) fn upload(&self, sources: Vec<PathBuf>, policy: CollisionPolicy) -> TransferId {
        self.runtime
            .enqueue(UploadRequest {
                sources,
                host: nocterm_session::HostAddress::direct(Target::new("u", "pinned-host", 2222)),
                destination: "/dest".into(),
                fs: self.remote.clone(),
                collisions: policy,
            })
            .unwrap()
    }
    pub(super) fn download(&self, sources: Vec<String>, policy: CollisionPolicy) -> TransferId {
        self.runtime
            .enqueue_download(DownloadRequest {
                sources,
                host: nocterm_session::HostAddress::direct(Target::new("u", "pinned-host", 2222)),
                local_destination: native("output"),
                fs: self.remote.clone(),
                collisions: policy,
            })
            .unwrap()
    }
    pub(super) fn finished(&self, id: TransferId) -> Progress {
        wait(|| {
            self.runtime
                .snapshot()
                .iter()
                .any(|progress| progress.id == id && progress.state.finished())
        });
        self.runtime
            .snapshot()
            .into_iter()
            .find(|progress| progress.id == id)
            .unwrap()
    }
    pub(super) fn local_bytes(&self, name: &str) -> Vec<u8> {
        let mut reader =
            LocalFs::open_source(self.local.as_ref(), &native(&format!("output/{name}"))).unwrap();
        let mut bytes = Vec::new();
        loop {
            let chunk = reader.read(32768).unwrap();
            if chunk.is_empty() {
                break;
            }
            bytes.extend(chunk);
        }
        reader.close().unwrap();
        bytes
    }
}
pub(super) fn native(path: &str) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from("C:\\").join(path)
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/").join(path)
    }
}
pub(super) fn wait(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "Transfer behavior did not settle"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

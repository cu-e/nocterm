use super::*;
use std::{
    io::{BufRead as _, BufReader},
    process::{Child, Command, Stdio},
    sync::atomic::AtomicUsize,
};
use zbus::{connection::Builder, object_server::SignalEmitter, zvariant::OwnedObjectPath};
struct Bus {
    _config: std::path::PathBuf,
    child: Child,
    address: String,
}
impl Bus {
    fn new() -> Self {
        static NEXT_BUS: AtomicUsize = AtomicUsize::new(0);
        let config = std::env::temp_dir().join(format!(
            "nocterm-broker-test-{}-{}.conf",
            std::process::id(),
            NEXT_BUS.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&config, r#"<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth><policy context="default"><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/></policy></busconfig>"#).unwrap();
        let mut child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon is required for broker protocol tests");
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        assert!(
            !address.trim().is_empty(),
            "private dbus-daemon did not start"
        );
        Self {
            _config: config,
            child,
            address: address.trim().into(),
        }
    }
}
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self._config);
    }
}
struct Manager;
#[zbus::interface(name = "net.reactivated.Fprint.Manager")]
impl Manager {
    fn get_devices(&self) -> Vec<OwnedObjectPath> {
        vec![OwnedObjectPath::try_from("/net/reactivated/Fprint/Device/0").unwrap()]
    }
}
type ScanScript = Vec<(String, bool)>;
struct Device {
    calls: Arc<AtomicUsize>,
    mode: Arc<AtomicUsize>,
    scripts: Arc<std::sync::Mutex<std::collections::VecDeque<ScanScript>>>,
    events: Arc<std::sync::Mutex<Vec<String>>>,
    cancel: Arc<AtomicBool>,
}
#[zbus::interface(name = "net.reactivated.Fprint.Device")]
impl Device {
    fn list_enrolled_fingers(&self, username: &str) -> Vec<String> {
        if username.is_empty() {
            vec![]
        } else {
            vec!["left-index-finger".into()]
        }
    }
    async fn claim(
        &self,
        _username: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        // A completion from a previous claim can still arrive while a new
        // caller is claiming the reader. It is not proof of this scan.
        if self.mode.load(Ordering::SeqCst) == 3 {
            Self::verify_status(&emitter, "verify-match", true)
                .await
                .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        }
        Ok(())
    }
    fn release(&self) {
        self.events.lock().unwrap().push("release".into());
    }
    async fn verify_stop(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        self.events.lock().unwrap().push("stop".into());
        if self.mode.load(Ordering::SeqCst) == 5 {
            return Err(fdo::Error::Failed(
                "Fingerprint reader could not stop scanning".into(),
            ));
        }
        if self.mode.load(Ordering::SeqCst) == 7 {
            self.cancel.store(true, Ordering::SeqCst);
        }
        // Stale success emitted during stop must not authorize the next scan.
        Self::verify_status(&emitter, "verify-match", true)
            .await
            .map_err(denied)
    }
    async fn verify_start(
        &self,
        _finger: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.events.lock().unwrap().push("start".into());
        if self.mode.load(Ordering::SeqCst) == 4 {
            return Err(fdo::Error::Failed(
                "Fingerprint reader could not start scanning".into(),
            ));
        }
        if self.mode.load(Ordering::SeqCst) == 6 {
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        let script = self.scripts.lock().unwrap().pop_front();
        if let Some(script) = script {
            for (status, done) in script {
                Self::verify_status(&emitter, &status, done)
                    .await
                    .map_err(denied)?;
            }
            return Ok(());
        }
        let result = match self.mode.load(Ordering::SeqCst) {
            0 => Self::verify_status(&emitter, "verify-match", true).await,
            1 | 3 => Self::verify_status(&emitter, "verify-no-match", true).await,
            _ => Ok(()),
        };
        result.map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }
    #[zbus(signal)]
    async fn verify_status(
        emitter: &SignalEmitter<'_>,
        status: &str,
        done: bool,
    ) -> zbus::Result<()>;
}
async fn connect(bus: &Bus) -> Connection {
    Builder::address(bus.address.as_str())
        .unwrap()
        .build()
        .await
        .unwrap()
}
#[tokio::test]
async fn key_expiry_runs_while_unrelated_name_ownership_keeps_changing() {
    let bus = Bus::new();
    let service = connect(&bus).await;
    let client = connect(&bus).await;
    let churner = connect(&bus).await;
    let owner = Owner {
        uid: nix::unistd::getuid().as_raw(),
        connection: client.unique_name().unwrap().to_string(),
    };
    let binding = "a".repeat(64);
    let store = Arc::new(Mutex::new(Store::default()));
    let token = store
        .lock()
        .await
        .enroll(
            owner.clone(),
            binding.clone(),
            zeroize::Zeroizing::new([42; 32]),
        )
        .unwrap();
    let observe_store = store.clone();
    let observer = tokio::spawn(async move {
        observe_connections(service, observe_store, Duration::from_millis(30))
            .await
            .unwrap();
    });
    let events = Arc::new(AtomicUsize::new(0));
    let produced = events.clone();
    let traffic = tokio::spawn(async move {
        loop {
            churner
                .request_name("dev.nocterm.ExpiryChurn")
                .await
                .unwrap();
            churner
                .release_name("dev.nocterm.ExpiryChurn")
                .await
                .unwrap();
            produced.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(3)).await;
        }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while events.load(Ordering::SeqCst) < 5 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    // Age the registration after the observer and real bus traffic started.
    // No further Store::entry call can perform opportunistic expiration.
    let cancelled = {
        let mut store = store.lock().await;
        let entry = store.entry(&owner, &binding, &token).unwrap();
        entry.touched -= Duration::from_secs(9 * 60 * 60);
        entry.cancel.clone()
    };
    let expired = tokio::time::timeout(Duration::from_millis(750), async {
        while !cancelled.load(Ordering::SeqCst) || events.load(Ordering::SeqCst) <= 5 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await;
    let observed_events = events.load(Ordering::SeqCst);
    traffic.abort();
    observer.abort();
    let _ = traffic.await;
    let _ = observer.await;
    assert!(expired.is_ok(), "bus traffic prevented expired key erasure");
    assert!(
        observed_events > 5,
        "the bus stopped changing before key expiry"
    );
}
#[tokio::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
async fn private_bus_requires_actual_verification_before_key_release() {
    let bus = Bus::new();
    let service = connect(&bus).await;
    let fake = connect(&bus).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let mode = Arc::new(AtomicUsize::new(0));
    fake.object_server()
        .at("/net/reactivated/Fprint/Manager", Manager)
        .await
        .unwrap();
    fake.object_server()
        .at(
            "/net/reactivated/Fprint/Device/0",
            Device {
                calls: calls.clone(),
                mode: mode.clone(),
                scripts: Default::default(),
                events: Default::default(),
                cancel: Default::default(),
            },
        )
        .await
        .unwrap();
    fake.request_name(FPRINT).await.unwrap();
    let store = Arc::new(Mutex::new(Store::default()));
    service
        .object_server()
        .at(
            PATH,
            Broker {
                store: store.clone(),
                scanner: Mutex::new(()),
                connection: service.clone(),
                fingerprint_uid: nix::unistd::getuid().as_raw(),
                scan_timeout: Duration::from_secs(30),
            },
        )
        .await
        .unwrap();
    service.request_name(NAME).await.unwrap();
    let client = connect(&bus).await;
    let other = connect(&bus).await;
    let owner = Proxy::new(&client, NAME, PATH, NAME).await.unwrap();
    let restarted = Proxy::new(&other, NAME, PATH, NAME).await.unwrap();
    let binding = "a".repeat(64);
    let key = [42u8; 32];
    let mut token: String = owner
        .call("Enroll", &(binding.as_str(), key.as_slice()))
        .await
        .unwrap();
    // A restarted Nocterm (another connection of the same user) still
    // finds the key, but only fingerprint verification releases it.
    let registered: bool = restarted
        .call("Registered", &(binding.as_str(), token.as_str()))
        .await
        .unwrap();
    assert!(registered);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let actual: Vec<u8> = owner
        .call("Release", &(binding.as_str(), token.as_str()))
        .await
        .unwrap();
    assert_eq!(actual, key);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    mode.store(1, Ordering::SeqCst);
    let attempt: Result<Vec<u8>, _> = owner
        .call("Release", &(binding.as_str(), token.as_str()))
        .await;
    assert!(attempt.is_err());
    let remaining: u8 = restarted
        .call("AttemptsRemaining", &(binding.as_str(), token.as_str()))
        .await
        .unwrap();
    assert_eq!(remaining, 0);
    let calls_before = calls.load(Ordering::SeqCst);
    let attempt: Result<Vec<u8>, _> = restarted
        .call("Release", &(binding.as_str(), token.as_str()))
        .await;
    assert!(
        matches!(attempt, Err(zbus::Error::MethodError(name, _, _)) if name.as_str() == "dev.nocterm.VaultBroker1.Error.Locked")
    );
    assert_eq!(calls.load(Ordering::SeqCst), calls_before);
    let unauthorized: Result<u8, _> = owner
        .call("AttemptsRemaining", &(binding.as_str(), "b".repeat(64)))
        .await;
    assert!(unauthorized.is_err());
    token = owner
        .call("Enroll", &(binding.as_str(), key.as_slice()))
        .await
        .unwrap();
    mode.store(3, Ordering::SeqCst);
    let attempt: Result<Vec<u8>, _> = owner
        .call("Release", &(binding.as_str(), token.as_str()))
        .await;
    assert!(
        attempt.is_err(),
        "a previous claim's successful signal cannot authorize a failed fresh scan"
    );
    token = owner
        .call("Enroll", &(binding.as_str(), key.as_slice()))
        .await
        .unwrap();
    mode.store(2, Ordering::SeqCst);
    let args = (binding.as_str(), token.as_str());
    let pending = owner.call::<_, _, Vec<u8>>("Release", &args);
    tokio::pin!(pending);
    tokio::select! {_=&mut pending=>panic!("verification released without fingerprint"),_=tokio::time::sleep(Duration::from_millis(100))=>{}}
    other
        .emit_signal(
            None::<&str>,
            "/net/reactivated/Fprint/Device/0",
            "net.reactivated.Fprint.Device",
            "VerifyStatus",
            &("verify-match", true),
        )
        .await
        .unwrap();
    tokio::select! {_=&mut pending=>panic!("forged fingerprint signal released the key"),_=tokio::time::sleep(Duration::from_millis(100))=>{}}
    let _: () = owner.call("Cancel", &args).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), pending)
            .await
            .unwrap()
            .is_err()
    );
    let _: () = owner.call("Remove", &args).await.unwrap();
    let registered: bool = owner.call("Registered", &args).await.unwrap();
    assert!(!registered);
}

mod retries;

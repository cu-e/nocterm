use crate::store::{Owner, Store};
use futures::StreamExt as _;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Mutex;
use zbus::{
    Connection, Proxy,
    fdo::{self, DBusProxy},
    message::Header,
    names::BusName,
};
const NAME: &str = "dev.nocterm.VaultBroker1";
const PATH: &str = "/dev/nocterm/VaultBroker1";
const FPRINT: &str = "net.reactivated.Fprint";
struct Broker {
    store: Arc<Mutex<Store>>,
    scanner: Mutex<()>,
    connection: Connection,
    #[cfg(test)]
    fingerprint_uid: u32,
}
fn denied(error: impl std::fmt::Display) -> fdo::Error {
    fdo::Error::AccessDenied(error.to_string())
}
impl Broker {
    fn fingerprint_uid(&self) -> u32 {
        #[cfg(test)]
        {
            self.fingerprint_uid
        }
        #[cfg(not(test))]
        {
            0
        }
    }
    async fn owner(&self, header: &Header<'_>) -> fdo::Result<Owner> {
        let sender = header.sender().ok_or_else(|| denied("Missing sender"))?;
        let dbus = DBusProxy::new(&self.connection).await.map_err(denied)?;
        let uid = dbus
            .get_connection_unix_user(BusName::from(sender.clone()))
            .await
            .map_err(denied)?;
        if uid == 0 {
            return Err(denied("Vault registrations require a desktop user"));
        }
        Ok(Owner {
            uid,
            connection: sender.to_string(),
        })
    }
    async fn verify(&self, owner: &Owner, cancel: Arc<AtomicBool>) -> fdo::Result<()> {
        let _scanner = self
            .scanner
            .try_lock()
            .map_err(|_| fdo::Error::Failed("Fingerprint reader is busy".into()))?;
        let mut claimed = None;
        let result = tokio::time::timeout(Duration::from_secs(35), async {
            let dbus = DBusProxy::new(&self.connection).await.map_err(denied)?;
            let _ = dbus.start_service_by_name(zbus::names::WellKnownName::try_from(FPRINT).map_err(denied)?, 0).await;
            let name = dbus
                .get_name_owner(BusName::try_from(FPRINT).map_err(denied)?)
                .await
                .map_err(denied)?;
            if dbus
                .get_connection_unix_user(BusName::from(name.clone()))
                .await
                .map_err(denied)?
                != self.fingerprint_uid()
            {
                return Err(denied("Untrusted fingerprint service"));
            }
            let user = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(owner.uid))
                .map_err(denied)?
                .ok_or_else(|| denied("Unknown desktop user"))?;
            let manager = Proxy::new(
                &self.connection,
                name.clone(),
                "/net/reactivated/Fprint/Manager",
                "net.reactivated.Fprint.Manager",
            )
            .await
            .map_err(denied)?;
            let devices: Vec<zbus::zvariant::OwnedObjectPath> =
                manager.call("GetDevices", &()).await.map_err(denied)?;
            let mut chosen = None;
            if devices.len() > 16 {
                return Err(denied("Too many fingerprint devices"));
            }
            for path in devices {
                let device = Proxy::new(
                    &self.connection,
                    name.clone(),
                    path,
                    "net.reactivated.Fprint.Device",
                )
                .await
                .map_err(denied)?;
                let fingers: Result<Vec<String>, _> = device
                    .call("ListEnrolledFingers", &(user.name.as_str(),))
                    .await;
                if fingers.is_ok_and(|f| !f.is_empty() && f.len() <= 10) {
                    chosen = Some(device);
                    break;
                }
            }
            let device = chosen.ok_or_else(|| denied("No enrolled fingerprint reader"))?;
            let _: () = device
                .call("Claim", &(user.name.as_str(),))
                .await
                .map_err(denied)?;
            claimed = Some(device.clone());
            // Claim is the exclusive boundary between verification owners.
            // Subscribe afterwards to exclude a previous owner's completion,
            // but before VerifyStart to retain synchronous fresh results.
            let mut statuses = device
                .receive_signal("VerifyStatus")
                .await
                .map_err(denied)?;
            if cancel.load(Ordering::SeqCst) {
                return Err(denied("Authentication cancelled"));
            }
            let _: () = device.call("VerifyStart", &("any",)).await.map_err(denied)?;
            let deadline = tokio::time::sleep(Duration::from_secs(30));
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    _ = &mut deadline => return Err(denied("Fingerprint authentication timed out")),
                    _ = tokio::time::sleep(Duration::from_millis(50)) => {
                        if cancel.load(Ordering::SeqCst) {
                            return Err(denied("Authentication cancelled"));
                        }
                        let client = BusName::try_from(owner.connection.as_str()).map_err(denied)?;
                        if dbus.get_connection_unix_user(client).await.is_err() {
                            return Err(denied("Client disconnected"));
                        }
                    }
                    message = statuses.next() => {
                        let message = message.ok_or_else(|| denied("Fingerprint service disconnected"))?;
                        if message.header().sender() != Some(&name) {
                            return Err(denied("Untrusted fingerprint signal"));
                        }
                        let (status, done): (String, bool) = message.body().deserialize().map_err(denied)?;
                        if cancel.load(Ordering::SeqCst) {
                            return Err(denied("Authentication cancelled"));
                        }
                        if status == "verify-match" && done {
                            return Ok(());
                        }
                        if done {
                            return Err(denied("Fingerprint was not recognized"));
                        }
                    }
                }
            }
        })
        .await
        .unwrap_or_else(|_| Err(denied("Fingerprint service timed out")));
        // Cleanup is also bounded when a service disconnects during verification.
        if let Some(device) = claimed {
            let _: Result<Result<(), zbus::Error>, _> =
                tokio::time::timeout(Duration::from_secs(1), device.call("VerifyStop", &())).await;
            let _: Result<Result<(), zbus::Error>, _> =
                tokio::time::timeout(Duration::from_secs(1), device.call("Release", &())).await;
        }
        result
    }
}
#[zbus::interface(name = "dev.nocterm.VaultBroker1")]
impl Broker {
    async fn enroll(
        &self,
        binding: String,
        key: Vec<u8>,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<String> {
        let key = zeroize::Zeroizing::new(key);
        if key.len() != 32 {
            return Err(denied("Invalid key length"));
        }
        let owner = self.owner(&header).await?;
        let mut secret = zeroize::Zeroizing::new([0; 32]);
        secret.copy_from_slice(&key);
        self.store
            .lock()
            .await
            .enroll(owner, binding, secret)
            .map_err(denied)
    }
    async fn registered(
        &self,
        binding: String,
        token: String,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<bool> {
        let owner = self.owner(&header).await?;
        Ok(self
            .store
            .lock()
            .await
            .entry(&owner, &binding, &token)
            .is_ok())
    }
    async fn release(
        &self,
        binding: String,
        token: String,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<Vec<u8>> {
        let owner = self.owner(&header).await?;
        let cancel = {
            let mut store = self.store.lock().await;
            let entry = store.entry(&owner, &binding, &token).map_err(denied)?;
            if entry.busy {
                return Err(denied("Authentication is already in progress"));
            }
            entry.busy = true;
            // A disconnecting caller cancels only its own authentication.
            entry.owner = owner.clone();
            entry.cancel = Arc::new(AtomicBool::new(false));
            entry.cancel.clone()
        };
        let result = self.verify(&owner, cancel.clone()).await;
        let mut store = self.store.lock().await;
        let entry = store.entry(&owner, &binding, &token).map_err(denied)?;
        entry.busy = false;
        result?;
        if cancel.load(Ordering::SeqCst) {
            return Err(denied("Authentication cancelled"));
        }
        entry.touched = std::time::Instant::now();
        Ok(entry.key.to_vec())
    }
    async fn cancel(
        &self,
        binding: String,
        token: String,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        let owner = self.owner(&header).await?;
        self.store
            .lock()
            .await
            .entry(&owner, &binding, &token)
            .map_err(denied)?
            .cancel
            .store(true, Ordering::SeqCst);
        Ok(())
    }
    async fn remove(
        &self,
        binding: String,
        token: String,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        let owner = self.owner(&header).await?;
        self.store
            .lock()
            .await
            .remove(&owner, &binding, &token)
            .map_err(denied)
    }
}
pub(crate) async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if nix::unistd::getuid().as_raw() != 0 {
        return Err("Install the root-owned vault broker service; do not run the desktop application as root".into());
    }
    let connection = zbus::connection::Builder::address("unix:path=/run/dbus/system_bus_socket")?
        .method_timeout(Duration::from_secs(5))
        .build()
        .await?;
    let store = Arc::new(Mutex::new(Store::default()));
    connection
        .object_server()
        .at(
            PATH,
            Broker {
                store: store.clone(),
                scanner: Mutex::new(()),
                connection: connection.clone(),
                #[cfg(test)]
                fingerprint_uid: 0,
            },
        )
        .await?;
    connection.request_name(NAME).await?;
    observe_connections(connection, store, Duration::from_secs(30)).await
}
async fn observe_connections(
    connection: Connection,
    store: Arc<Mutex<Store>>,
    expiry_period: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let dbus = DBusProxy::new(&connection).await?;
    let mut changes = dbus.receive_name_owner_changed().await?;
    let mut expiry = tokio::time::interval(expiry_period);
    expiry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            change = changes.next() => {
                let Some(change) = change else { break; };
                let args = change.args()?;
                if args.new_owner().is_none() {
                    store.lock().await.disconnected(args.name().as_str());
                }
            }
            _ = expiry.tick() => store.lock().await.expire(),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
    struct Device {
        calls: Arc<AtomicUsize>,
        mode: Arc<AtomicUsize>,
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
        fn release(&self) {}
        fn verify_stop(&self) {}
        async fn verify_start(
            &self,
            _finger: &str,
            #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        ) -> zbus::fdo::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
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
        let token: String = owner
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
        mode.store(3, Ordering::SeqCst);
        let attempt: Result<Vec<u8>, _> = owner
            .call("Release", &(binding.as_str(), token.as_str()))
            .await;
        assert!(
            attempt.is_err(),
            "a previous claim's successful signal cannot authorize a failed fresh scan"
        );
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
}

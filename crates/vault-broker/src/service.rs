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
    #[cfg(test)]
    scan_timeout: Duration,
}
fn denied(error: impl std::fmt::Display) -> fdo::Error {
    fdo::Error::AccessDenied(error.to_string())
}
enum ScanStatus {
    Pending,
    Match,
    NoMatch,
}
fn verification_status(status: &str, done: bool) -> fdo::Result<ScanStatus> {
    match status {
        "verify-match" if done => Ok(ScanStatus::Match),
        "verify-no-match" if done => Ok(ScanStatus::NoMatch),
        "verify-disconnected" => Err(denied("Fingerprint reader disconnected")),
        "verify-unknown-error" => Err(denied("Fingerprint reader reported an unknown error")),
        _ if !done => Ok(ScanStatus::Pending),
        "verify-retry-scan" => Err(denied("Fingerprint scan must be retried")),
        "verify-swipe-too-short" => Err(denied("Fingerprint swipe was too short")),
        "verify-finger-not-centered" => Err(denied("Finger was not centered on the reader")),
        "verify-remove-and-retry" => Err(denied("Remove your finger and try again")),
        "verify-too-fast" => Err(denied("Finger moved too quickly")),
        _ => Err(denied(format!("Fingerprint verification failed: {status}"))),
    }
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
    fn scan_timeout(&self) -> Duration {
        #[cfg(test)]
        {
            self.scan_timeout
        }
        #[cfg(not(test))]
        {
            Duration::from_secs(30)
        }
    }
    async fn verify(&self, owner: &Owner, cancel: Arc<AtomicBool>) -> fdo::Result<()> {
        let _scanner = self
            .scanner
            .try_lock()
            .map_err(|_| fdo::Error::Failed("Fingerprint reader is busy".into()))?;
        let mut claimed = None;
        let mut disconnected = false;
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
            // One deadline covers all scans and retry setup. Claim separates owners;
            // each new subscription separates completed attempts.
            let deadline = tokio::time::sleep(self.scan_timeout());
            tokio::pin!(deadline);
            for attempt in 1..=3 {
                let scan = async {
                    if cancel.load(Ordering::SeqCst) {
                        return Err(denied("Authentication cancelled"));
                    }
                    let mut statuses = device.receive_signal("VerifyStatus").await.map_err(denied)?;
                    let _: () = device.call("VerifyStart", &("any",)).await.map_err(denied)?;
                    loop {
                        tokio::select! {
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
                                disconnected |= status == "verify-disconnected";
                                match verification_status(&status, done)? {
                                    ScanStatus::Pending => {}
                                    ScanStatus::Match => return Ok(true),
                                    ScanStatus::NoMatch => return Ok(false),
                                }
                            }
                        }
                    }
                };
                let matched = tokio::select! {
                    _ = &mut deadline => return Err(denied("Fingerprint authentication timed out")),
                    result = scan => result?,
                };
                if matched {
                    return Ok(());
                }
                if attempt == 3 {
                    return Err(denied("Fingerprint was not recognized after 3 attempts"));
                }
                // The old stream is dropped before stopping. Subscribe afresh only
                // after VerifyStop finishes, excluding stale completion signals.
                tokio::select! {
                    _ = &mut deadline => return Err(denied("Fingerprint authentication timed out")),
                    result = device.call::<_, _, ()>("VerifyStop", &()) => result.map_err(denied)?,
                }
            }
            unreachable!("three unsuccessful attempts return an error")
        })
        .await
        .unwrap_or_else(|_| Err(denied("Fingerprint service timed out")));
        // Cleanup is also bounded when a service disconnects during verification.
        if let Some(device) = claimed.filter(|_| !disconnected) {
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
                #[cfg(test)]
                scan_timeout: Duration::from_secs(30),
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
mod tests;

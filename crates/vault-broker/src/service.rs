mod fingerprint;
mod fprintd;
mod verify;

#[cfg(test)]
use self::fprintd::FPRINT;
use self::fprintd::FprintdBackend;
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
    Connection,
    fdo::{self, DBusProxy},
    message::Header,
    names::BusName,
};
const NAME: &str = "dev.nocterm.VaultBroker1";
const PATH: &str = "/dev/nocterm/VaultBroker1";
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
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "dev.nocterm.VaultBroker1.Error")]
enum LockoutError {
    Locked(String),
}
#[derive(Debug)]
enum ReleaseError {
    Fdo(fdo::Error),
    Locked(LockoutError),
}
impl From<fdo::Error> for ReleaseError {
    fn from(error: fdo::Error) -> Self {
        Self::Fdo(error)
    }
}
impl zbus::DBusError for ReleaseError {
    fn create_reply(&self, header: &Header<'_>) -> zbus::Result<zbus::Message> {
        match self {
            Self::Fdo(error) => error.create_reply(header),
            Self::Locked(error) => error.create_reply(header),
        }
    }
    fn name(&self) -> zbus::names::ErrorName<'_> {
        match self {
            Self::Fdo(error) => error.name(),
            Self::Locked(error) => error.name(),
        }
    }
    fn description(&self) -> Option<&str> {
        match self {
            Self::Fdo(error) => error.description(),
            Self::Locked(error) => error.description(),
        }
    }
}
fn locked() -> ReleaseError {
    ReleaseError::Locked(LockoutError::Locked(
        "Fingerprint unlock is locked after 3 failed attempts. Unlock with the master password."
            .into(),
    ))
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
    async fn verify(
        &self,
        owner: &Owner,
        binding: &str,
        token: &str,
        cancel: Arc<AtomicBool>,
    ) -> fdo::Result<()> {
        let backend = FprintdBackend::new(&self.connection, self.fingerprint_uid()).await?;
        let request = verify::Request {
            attempts: verify::Attempts {
                store: &self.store,
                owner,
                binding,
                token,
            },
            scanner: &self.scanner,
            cancel: &cancel,
            scan_timeout: self.scan_timeout(),
        };
        verify::verify(&backend, &request).await
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
    async fn attempts_remaining(
        &self,
        binding: String,
        token: String,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<u8> {
        let owner = self.owner(&header).await?;
        Ok(self
            .store
            .lock()
            .await
            .entry(&owner, &binding, &token)
            .map_err(denied)?
            .attempts_remaining)
    }
    async fn release(
        &self,
        binding: String,
        token: String,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Vec<u8>, ReleaseError> {
        let owner = self.owner(&header).await?;
        let cancel = {
            let mut store = self.store.lock().await;
            let entry = store.entry(&owner, &binding, &token).map_err(denied)?;
            if entry.attempts_remaining == 0 {
                return Err(locked());
            }
            if entry.busy {
                return Err(denied("Authentication is already in progress").into());
            }
            entry.busy = true;
            // A disconnecting caller cancels only its own authentication.
            entry.owner = owner.clone();
            entry.cancel = Arc::new(AtomicBool::new(false));
            entry.cancel.clone()
        };
        let result = self.verify(&owner, &binding, &token, cancel.clone()).await;
        let mut store = self.store.lock().await;
        let entry = store.entry(&owner, &binding, &token).map_err(denied)?;
        entry.busy = false;
        if result.is_err() && entry.attempts_remaining == 0 {
            return Err(locked());
        }
        result?;
        if cancel.load(Ordering::SeqCst) {
            return Err(denied("Authentication cancelled").into());
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

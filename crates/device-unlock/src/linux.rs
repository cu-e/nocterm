//! Linux releases a session-only KEK through a root-owned broker, never keyring.
use crate::platform;
use nocterm_vault::{
    DeviceAvailability as A, DeviceCancellation, DeviceCapability, DeviceUnlockError as E,
    DeviceUnlockProvider, VaultBinding, VaultKey,
};
use std::{sync::Mutex, time::Duration};
use zbus::{
    Connection, Proxy,
    fdo::DBusProxy,
    names::{BusName, WellKnownName},
};
const BROKER: &str = "dev.nocterm.VaultBroker1";
const PATH: &str = "/dev/nocterm/VaultBroker1";
const FPRINT: &str = "net.reactivated.Fprint";
struct Client {
    runtime: tokio::runtime::Runtime,
    connection: Connection,
}
#[derive(Default)]
pub(super) struct Linux {
    client: Mutex<Option<Client>>,
}
impl Client {
    fn bounded<T>(
        &self,
        timeout: Duration,
        future: impl std::future::Future<Output = Result<T, E>>,
    ) -> Result<T, E> {
        self.runtime.block_on(async {
            tokio::time::timeout(timeout, future)
                .await
                .map_err(|_| E::Platform("System authentication service timed out".into()))?
        })
    }
}
impl Linux {
    fn run<T>(&self, op: impl FnOnce(&Client) -> Result<T, E>) -> Result<T, E> {
        let mut guard = self.client.lock().map_err(platform)?;
        if guard.is_none() {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .map_err(platform)?;
            let connection = runtime
                .block_on(async {
                    tokio::time::timeout(
                        Duration::from_secs(5),
                        zbus::connection::Builder::address(
                            "unix:path=/run/dbus/system_bus_socket",
                        )?
                        .method_timeout(Duration::from_secs(40))
                        .build(),
                    )
                    .await
                    .map_err(|_| zbus::Error::Failure("Bus connection timed out".into()))?
                })
                .map_err(platform)?;
            *guard = Some(Client {
                runtime,
                connection,
            });
        }
        op(guard.as_ref().expect("initialized client"))
    }
}
async fn trusted(connection: &Connection, name: &str) -> Result<(), E> {
    let dbus = DBusProxy::new(connection).await.map_err(platform)?;
    if name == FPRINT {
        // fprintd exits while idle. D-Bus activation lists capabilities without
        // requesting authentication, then its authenticated owner is checked.
        let _ = dbus
            .start_service_by_name(WellKnownName::try_from(name).map_err(platform)?, 0)
            .await;
    }
    let owner = dbus
        .get_name_owner(BusName::try_from(name).map_err(platform)?)
        .await
        .map_err(platform)?;
    let uid = dbus
        .get_connection_unix_user(BusName::from(owner))
        .await
        .map_err(platform)?;
    if uid != 0 {
        return Err(E::Unavailable);
    }
    Ok(())
}
fn token_string(token: &[u8]) -> Result<&str, E> {
    let token = std::str::from_utf8(token).map_err(|_| E::Invalidated)?;
    if token.len() != 64
        || !token
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(E::Invalidated);
    }
    Ok(token)
}

impl DeviceUnlockProvider for Linux {
    fn probe(&self) -> Result<DeviceCapability, E> {
        let result = self.run(|client| {
            client.bounded(Duration::from_secs(8), async {
                trusted(&client.connection, FPRINT).await?;
                let manager = Proxy::new(
                    &client.connection,
                    FPRINT,
                    "/net/reactivated/Fprint/Manager",
                    "net.reactivated.Fprint.Manager",
                ).await.map_err(platform)?;
                let devices: Vec<zbus::zvariant::OwnedObjectPath> = manager
                    .call("GetDevices", &()).await.map_err(platform)?;
                if devices.is_empty() {
                    return Ok((A::NoHardware, "No supported fingerprint reader was found.".into()));
                }
                if devices.len() > 16 {
                    return Err(E::Unavailable);
                }
                let user = nix::unistd::User::from_uid(nix::unistd::getuid())
                    .map_err(platform)?.ok_or(E::Unavailable)?;
                let mut enrolled = false;
                for device in devices {
                    let proxy = Proxy::new(&client.connection, FPRINT, device,
                        "net.reactivated.Fprint.Device").await.map_err(platform)?;
                    let fingers: Result<Vec<String>, _> = proxy
                        .call("ListEnrolledFingers", &(user.name.as_str(),)).await;
                    enrolled |= fingers.is_ok_and(|fingers| !fingers.is_empty() && fingers.len() <= 10);
                }
                if !enrolled {
                    return Ok((A::NotEnrolled, "Register a fingerprint in your operating system settings.".into()));
                }
                if trusted(&client.connection, BROKER).await.is_err() {
                    return Ok((A::BrokerMissing, "Fingerprint reader detected. Install the Nocterm vault broker using scripts/install-vault-broker.sh; see docs/DEVICE_UNLOCK.md. The master password always works.".into()));
                }
                Ok((A::Available, "Unlock once with the master password after starting Nocterm. Fingerprint unlock then works until you exit.".into()))
            })
        });
        let (availability, detail) = result.unwrap_or((
            A::Unavailable,
            "Fingerprint services are unavailable. Use the master password.".into(),
        ));
        Ok(DeviceCapability {
            availability,
            label: "Fingerprint".into(),
            detail,
            enabled: false,
            session_only: true,
        })
    }
    fn enroll(
        &self,
        binding: VaultBinding,
        key: VaultKey,
        cancel: &DeviceCancellation,
    ) -> Result<Vec<u8>, E> {
        cancel.check()?;
        let token = self.run(|client| {
            client.bounded(Duration::from_secs(8), async {
                trusted(&client.connection, BROKER).await?;
                let proxy = Proxy::new(&client.connection, BROKER, PATH, BROKER)
                    .await
                    .map_err(platform)?;
                proxy
                    .call::<_, _, String>("Enroll", &(binding.identifier(), key.as_slice()))
                    .await
                    .map_err(platform)
            })
        })?;
        if let Err(error) = cancel.check() {
            let _ = self.remove(binding, token.as_bytes());
            return Err(error);
        }
        token_string(token.as_bytes())?;
        Ok(token.into_bytes())
    }
    fn release(
        &self,
        binding: VaultBinding,
        token: &[u8],
        cancel: &DeviceCancellation,
    ) -> Result<VaultKey, E> {
        let token = token_string(token)?;
        cancel.check()?;
        self.run(|client| client.bounded(Duration::from_secs(35), async {
            trusted(&client.connection, BROKER).await?;
            let proxy = Proxy::new(&client.connection,BROKER,PATH,BROKER).await.map_err(platform)?;
            let identity = binding.identifier();
            let arguments = (identity.as_str(), token);
            // Fingerprint authentication needs a longer budget than passive probes.
            let future = proxy.call::<_,_,Vec<u8>>("Release", &arguments);
            tokio::pin!(future);
            let bytes = loop {
                tokio::select! {
                    result = &mut future => break zeroize::Zeroizing::new(result.map_err(platform)?),
                    _ = tokio::time::sleep(Duration::from_millis(50)) => {
                        if cancel.is_cancelled() {
                            let _: Result<(),_> = tokio::time::timeout(Duration::from_secs(1), proxy.call("Cancel", &(identity.as_str(),token))).await.unwrap_or_else(|_| Err(zbus::Error::Failure("timeout".into())));
                            return Err(E::Cancelled);
                        }
                    }
                }
            };
            cancel.check()?;
            if bytes.len()!=32 { return Err(E::Invalidated); }
            let mut key = VaultKey::new([0;32]); key.copy_from_slice(&bytes); Ok(key)
        }))
    }
    fn remove(&self, binding: VaultBinding, token: &[u8]) -> Result<(), E> {
        let token = token_string(token)?;
        self.run(|client| {
            client.bounded(Duration::from_secs(8), async {
                // A terminated/restarted broker has already erased its session keys.
                if trusted(&client.connection, BROKER).await.is_err() {
                    return Ok(());
                }
                let proxy = Proxy::new(&client.connection, BROKER, PATH, BROKER)
                    .await
                    .map_err(platform)?;
                proxy
                    .call("Remove", &(binding.identifier(), token))
                    .await
                    .map_err(platform)
            })
        })
    }
    fn registered(&self, binding: VaultBinding, token: &[u8]) -> bool {
        let Ok(token) = token_string(token) else {
            return false;
        };
        self.run(|client| {
            client.bounded(Duration::from_secs(8), async {
                trusted(&client.connection, BROKER).await?;
                let proxy = Proxy::new(&client.connection, BROKER, PATH, BROKER)
                    .await
                    .map_err(platform)?;
                proxy
                    .call("Registered", &(binding.identifier(), token))
                    .await
                    .map_err(platform)
            })
        })
        .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn broker_tokens_are_bounded_canonical_identifiers() {
        for invalid in [
            vec![255; 64],
            b"../../vault".to_vec(),
            b"a".repeat(65),
            b"A".repeat(64),
        ] {
            assert!(token_string(&invalid).is_err());
        }
        assert_eq!(token_string(&b"a".repeat(64)).unwrap(), "a".repeat(64));
    }
}
